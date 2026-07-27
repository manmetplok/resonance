//! Time-stretch + independent pitch-shift processor (clip warp, doc #166).
//!
//! A single streaming processor that changes a signal's *length* and its
//! *pitch* independently:
//!
//! * `time_ratio` — output length ÷ input length (2.0 = twice as long,
//!   half speed; 0.5 = half as long, double speed) with pitch unchanged.
//! * `pitch_semitones` — frequency shift in semitones, decoupled from the
//!   stretch (formant correction is left to a later doc-#160 primitive,
//!   so this is a plain resampling shift for now).
//!
//! Two algorithms sit behind [`StretchAlgorithm`], chosen per the clip's
//! material:
//!
//! * [`StretchAlgorithm::Tonal`] — a phase-vocoder (STFT, per-bin phase
//!   propagation). Smoothest on sustained / harmonic material.
//! * [`StretchAlgorithm::Transient`] — WSOLA (waveform-similarity
//!   overlap-add). Preserves attacks on drums / percussive loops.
//!
//! # Pitch ⟂ stretch
//!
//! Pitch shifting is time-stretching followed by resampling: to shift by
//! `p` semitones the internal stretcher runs at `time_ratio · 2^(p/12)`
//! and the output is then resampled (read) at `2^(p/12)` samples per
//! output sample. The resampling restores the requested `time_ratio`
//! length while multiplying every frequency by `2^(p/12)`.
//!
//! # Streaming & determinism
//!
//! The processor is fed source samples with [`TimeStretch::feed`] and
//! produces stretched output with [`TimeStretch::pull`]; call
//! [`TimeStretch::finish`] at end-of-input to flush the tail. Frames are
//! consumed on the stretcher's *internal* hop grid, independent of how
//! the caller chunks `feed`/`pull`, so block-by-block live rendering and
//! a single offline pass over the whole clip produce **bitwise-identical
//! output** (the property the mixer relies on for matching playback and
//! bounce). Generation is a pure function of the input samples and the
//! parameter values — no RNG, no wall-clock, no global state.
//!
//! Parameters are sampled when a frame is formed; change them at block
//! boundaries (the mixer's natural cadence) for predictable results.

mod ola;
mod phase_vocoder;
mod wsola;

pub use ola::Ola;
pub use phase_vocoder::PhaseVocoder;
pub use wsola::Wsola;

/// Algorithm used by [`TimeStretch`]. Maps 1:1 onto the engine's
/// `WarpAlgorithm` (doc #166); kept separate so this crate stays free of
/// engine types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchAlgorithm {
    /// Phase-vocoder. Best for sustained / harmonic material.
    Tonal,
    /// WSOLA overlap-add. Best for percussive / transient material.
    Transient,
}

/// STFT / OLA frame size (samples). A power of two for the FFT.
pub const FRAME: usize = 1024;
/// Overlap factor: synthesis hop is `FRAME / OVERLAP` (75 % overlap).
pub const OVERLAP: usize = 4;
/// Synthesis hop (output advance per frame), in samples.
pub const SYNTH_HOP: usize = FRAME / OVERLAP;
/// WSOLA similarity-search radius around the nominal analysis position.
pub const WSOLA_SEARCH: usize = SYNTH_HOP / 2;
/// Floor applied to the OLA normalisation weight to avoid 0/0 at the
/// signal's leading/trailing edges where window coverage is partial.
pub const WEIGHT_EPS: f32 = 1e-6;

/// Clamp bounds for `time_ratio`. Extreme ratios are neither musically
/// useful nor numerically well-behaved (analysis hop → 0 or huge).
const MIN_TIME_RATIO: f32 = 0.1;
const MAX_TIME_RATIO: f32 = 10.0;
/// Clamp bounds for `pitch_semitones` (± four octaves).
const MAX_SEMITONES: f32 = 48.0;

/// A simple growable sample FIFO with O(1) amortised front-drop. Indexing
/// is relative to the current front (sample 0 = oldest unconsumed).
pub(crate) struct Fifo {
    buf: Vec<f32>,
    head: usize,
}

impl Fifo {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            head: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len() - self.head
    }

    pub fn push(&mut self, samples: &[f32]) {
        self.buf.extend_from_slice(samples);
    }

    pub fn get(&self, i: usize) -> f32 {
        self.buf[self.head + i]
    }

    pub fn consume(&mut self, n: usize) {
        self.head = (self.head + n).min(self.buf.len());
        // Compact when the dead prefix dominates, bounding memory while
        // keeping the common path allocation-free.
        if self.head > 1 << 16 && self.head * 2 >= self.buf.len() {
            self.buf.drain(..self.head);
            self.head = 0;
        }
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.head = 0;
    }
}

/// Streaming time-stretch + pitch-shift processor. See the module docs.
pub struct TimeStretch {
    algorithm: StretchAlgorithm,
    time_ratio: f32,
    pitch_semitones: f32,

    /// Input sample FIFO (source material, fed by the caller).
    input: Fifo,
    /// Output of the stretch stage, before pitch resampling.
    stretched: Fifo,

    /// Phase-vocoder state (used by [`StretchAlgorithm::Tonal`]).
    pv: PhaseVocoder,
    /// WSOLA state (used by [`StretchAlgorithm::Transient`]).
    wsola: Wsola,

    /// Fractional read position into `stretched` for the resampler.
    resample_pos: f64,
    /// True once `finish` has been called: the input is complete and the
    /// stretch tail has been flushed.
    finished: bool,
}

impl TimeStretch {
    /// Create a processor at `sample_rate` Hz using `algorithm`, with no
    /// stretch and no pitch shift (`time_ratio = 1`, `pitch = 0`).
    ///
    /// `sample_rate` is accepted for API symmetry with the rest of the
    /// crate and future formant work; the current algorithms are
    /// sample-rate-agnostic (everything is expressed in samples).
    pub fn new(_sample_rate: f32, algorithm: StretchAlgorithm) -> Self {
        Self {
            algorithm,
            time_ratio: 1.0,
            pitch_semitones: 0.0,
            input: Fifo::new(),
            stretched: Fifo::new(),
            pv: PhaseVocoder::new(),
            wsola: Wsola::new(),
            resample_pos: 0.0,
            finished: false,
        }
    }

    /// Output-length ÷ input-length ratio currently in effect.
    pub fn time_ratio(&self) -> f32 {
        self.time_ratio
    }

    /// Pitch shift in semitones currently in effect.
    pub fn pitch_semitones(&self) -> f32 {
        self.pitch_semitones
    }

    /// The selected algorithm.
    pub fn algorithm(&self) -> StretchAlgorithm {
        self.algorithm
    }

    /// Set the output ÷ input length ratio (clamped to a sane range).
    pub fn set_time_ratio(&mut self, ratio: f32) {
        self.time_ratio = clamp_finite(ratio, MIN_TIME_RATIO, MAX_TIME_RATIO, 1.0);
    }

    /// Set the pitch shift in semitones (clamped to ± four octaves).
    pub fn set_pitch_semitones(&mut self, semitones: f32) {
        self.pitch_semitones = clamp_finite(semitones, -MAX_SEMITONES, MAX_SEMITONES, 0.0);
    }

    /// Processing latency in samples: how many output samples of leading
    /// silence/priming precede the first sample that corresponds to input
    /// sample 0. The phase-vocoder must fill one analysis frame before it
    /// emits; WSOLA primes with one window. The caller compensates by
    /// discarding this many output samples (or shifting the timeline).
    pub fn latency(&self) -> usize {
        // Reported in output samples: the stretch-stage latency (a frame)
        // is consumed at the pitch resample rate.
        let frame_latency = FRAME as f64;
        (frame_latency / self.pitch_ratio() as f64).round() as usize
    }

    /// Clear all internal state, ready to process a fresh signal. Keeps
    /// the configured algorithm and parameters.
    pub fn reset(&mut self) {
        self.input.clear();
        self.stretched.clear();
        self.pv.reset();
        self.wsola.reset();
        self.resample_pos = 0.0;
        self.finished = false;
    }

    /// Push source samples into the processor.
    pub fn feed(&mut self, input: &[f32]) {
        debug_assert!(!self.finished, "feed called after finish");
        self.input.push(input);
    }

    /// Number of output samples currently available to [`pull`].
    ///
    /// [`pull`]: Self::pull
    pub fn available(&mut self) -> usize {
        self.run_stretch_stage();
        self.resample_available()
    }

    /// Fill `out` with stretched + pitch-shifted output, returning the
    /// number of samples written (may be fewer than `out.len()` when not
    /// enough input has been fed yet — feed more, or call [`finish`]).
    ///
    /// [`finish`]: Self::finish
    pub fn pull(&mut self, out: &mut [f32]) -> usize {
        self.run_stretch_stage();
        let mut written = 0;
        let rate = self.pitch_ratio() as f64;
        while written < out.len() {
            // Linear interpolation needs the sample at floor(pos)+1.
            let base = self.resample_pos.floor();
            let need = base as usize + 1;
            if need + 1 > self.stretched.len() {
                break;
            }
            let i0 = base as usize;
            let frac = (self.resample_pos - base) as f32;
            let s0 = self.stretched.get(i0);
            let s1 = self.stretched.get(i0 + 1);
            out[written] = s0 + (s1 - s0) * frac;
            written += 1;
            self.resample_pos += rate;
        }
        // Drop fully-consumed input from the stretched FIFO, keeping the
        // one sample straddled by the fractional read position.
        let consumed = self.resample_pos.floor() as usize;
        if consumed > 0 {
            self.stretched.consume(consumed);
            self.resample_pos -= consumed as f64;
        }
        written
    }

    /// Signal end-of-input and flush the stretcher's tail into the output
    /// FIFO so the final partial frames can be pulled. Idempotent.
    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.run_stretch_stage();
        // Pad the input with a frame of zeros so the last real samples
        // are covered by a full analysis window, then drain.
        self.input.push(&vec![0.0; FRAME]);
        self.run_stretch_stage();
        self.finished = true;
    }

    /// Convenience: stretch + pitch-shift a whole buffer in one call.
    /// Equivalent to `feed(input); finish(); pull(...)` until drained, so
    /// it returns exactly what block-by-block streaming would.
    pub fn process(
        sample_rate: f32,
        algorithm: StretchAlgorithm,
        time_ratio: f32,
        pitch_semitones: f32,
        input: &[f32],
    ) -> Vec<f32> {
        let mut ts = TimeStretch::new(sample_rate, algorithm);
        ts.set_time_ratio(time_ratio);
        ts.set_pitch_semitones(pitch_semitones);
        ts.feed(input);
        ts.finish();
        let mut out = Vec::new();
        let mut chunk = vec![0.0; 4096];
        loop {
            let n = ts.pull(&mut chunk);
            if n == 0 {
                break;
            }
            out.extend_from_slice(&chunk[..n]);
        }
        out
    }

    // -- internals ----------------------------------------------------

    /// 2^(semitones / 12): frequency multiplier and resample read rate.
    fn pitch_ratio(&self) -> f32 {
        2f32.powf(self.pitch_semitones / 12.0)
    }

    /// Total stretch the stretcher must apply so that, after resampling
    /// by `pitch_ratio`, the net length ratio is `time_ratio`.
    fn stretch_factor(&self) -> f32 {
        self.time_ratio * self.pitch_ratio()
    }

    fn resample_available(&self) -> usize {
        if self.stretched.len() < 2 {
            return 0;
        }
        let rate = self.pitch_ratio() as f64;
        // Last interpolable index is len-2 (needs +1 lookahead).
        let last = (self.stretched.len() - 2) as f64;
        if self.resample_pos > last {
            return 0;
        }
        (((last - self.resample_pos) / rate).floor() as usize) + 1
    }

    /// Drive the selected stretcher, draining as many frames as the
    /// buffered input allows into the `stretched` FIFO.
    fn run_stretch_stage(&mut self) {
        let factor = self.stretch_factor();
        match self.algorithm {
            StretchAlgorithm::Tonal => {
                self.pv.run(&mut self.input, &mut self.stretched, factor)
            }
            StretchAlgorithm::Transient => {
                self.wsola.run(&mut self.input, &mut self.stretched, factor)
            }
        }
    }
}

/// Clamp `value` to `[min, max]`, falling back to `default` if it is NaN
/// or infinite.
fn clamp_finite(value: f32, min: f32, max: f32, default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}
