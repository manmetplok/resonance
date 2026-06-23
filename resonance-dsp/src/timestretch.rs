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

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::window::hann_window;

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
const FRAME: usize = 1024;
/// Overlap factor: synthesis hop is `FRAME / OVERLAP` (75 % overlap).
const OVERLAP: usize = 4;
/// Synthesis hop (output advance per frame), in samples.
const SYNTH_HOP: usize = FRAME / OVERLAP;
/// WSOLA similarity-search radius around the nominal analysis position.
const WSOLA_SEARCH: usize = SYNTH_HOP / 2;
/// Floor applied to the OLA normalisation weight to avoid 0/0 at the
/// signal's leading/trailing edges where window coverage is partial.
const WEIGHT_EPS: f32 = 1e-6;

/// Clamp bounds for `time_ratio`. Extreme ratios are neither musically
/// useful nor numerically well-behaved (analysis hop → 0 or huge).
const MIN_TIME_RATIO: f32 = 0.1;
const MAX_TIME_RATIO: f32 = 10.0;
/// Clamp bounds for `pitch_semitones` (± four octaves).
const MAX_SEMITONES: f32 = 48.0;

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

// ---------------------------------------------------------------------------
// Input / output FIFO
// ---------------------------------------------------------------------------

/// A simple growable sample FIFO with O(1) amortised front-drop. Indexing
/// is relative to the current front (sample 0 = oldest unconsumed).
struct Fifo {
    buf: Vec<f32>,
    head: usize,
}

impl Fifo {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            head: 0,
        }
    }

    fn len(&self) -> usize {
        self.buf.len() - self.head
    }

    fn push(&mut self, samples: &[f32]) {
        self.buf.extend_from_slice(samples);
    }

    fn get(&self, i: usize) -> f32 {
        self.buf[self.head + i]
    }

    fn consume(&mut self, n: usize) {
        self.head = (self.head + n).min(self.buf.len());
        // Compact when the dead prefix dominates, bounding memory while
        // keeping the common path allocation-free.
        if self.head > 1 << 16 && self.head * 2 >= self.buf.len() {
            self.buf.drain(..self.head);
            self.head = 0;
        }
    }

    fn clear(&mut self) {
        self.buf.clear();
        self.head = 0;
    }
}

// ---------------------------------------------------------------------------
// Overlap-add accumulator (shared by both stretchers)
// ---------------------------------------------------------------------------

/// Fractional-hop overlap-add accumulator. Frames are added at rising
/// synthesis positions; each output sample is the windowed-frame sum
/// normalised by the accumulated window weight, so any (even fractional)
/// hop sequence reconstructs unity gain on stationary input.
struct Ola {
    signal: Vec<f32>,
    weight: Vec<f32>,
    /// Absolute synthesis index of `signal[0]` / `weight[0]`.
    base: usize,
    /// Fractional absolute position where the next frame is added.
    pos: f64,
    /// Absolute index up to which output has already been emitted.
    emitted: usize,
}

impl Ola {
    fn new() -> Self {
        Self {
            signal: Vec::new(),
            weight: Vec::new(),
            base: 0,
            pos: 0.0,
            emitted: 0,
        }
    }

    fn reset(&mut self) {
        self.signal.clear();
        self.weight.clear();
        self.base = 0;
        self.pos = 0.0;
        self.emitted = 0;
    }

    /// Add `frame` (already windowed) weighted by `window`, at the
    /// current synthesis position, then advance the position by `hop`.
    fn add_frame(&mut self, frame: &[f32], window: &[f32], hop: f64) {
        let start = self.pos.round() as usize;
        let rel = start - self.base;
        let end = rel + frame.len();
        if end > self.signal.len() {
            self.signal.resize(end, 0.0);
            self.weight.resize(end, 0.0);
        }
        for j in 0..frame.len() {
            self.signal[rel + j] += frame[j] * window[j];
            self.weight[rel + j] += window[j] * window[j];
        }
        self.pos += hop;
    }

    /// Emit every sample below `up_to` (absolute index) that will receive
    /// no further contributions, normalising by accumulated weight.
    fn drain_below(&mut self, up_to: usize, out: &mut Fifo) {
        if up_to <= self.emitted {
            return;
        }
        let from_rel = self.emitted - self.base;
        let to_rel = (up_to - self.base).min(self.signal.len());
        let mut tmp = Vec::with_capacity(to_rel.saturating_sub(from_rel));
        for k in from_rel..to_rel {
            let w = self.weight[k];
            tmp.push(if w > WEIGHT_EPS { self.signal[k] / w } else { 0.0 });
        }
        out.push(&tmp);
        let drained = to_rel - from_rel;
        self.emitted += drained;
        // Compact the consumed prefix.
        if from_rel + drained > 0 {
            self.signal.drain(..to_rel);
            self.weight.drain(..to_rel);
            self.base += to_rel;
        }
    }
}

// ---------------------------------------------------------------------------
// Phase vocoder (Tonal)
// ---------------------------------------------------------------------------

struct PhaseVocoder {
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    frame_buf: Vec<f32>,
    /// Per-bin analysis magnitude and phase for the current frame.
    mag: Vec<f32>,
    anal_phase: Vec<f32>,
    /// Previous analysis phase per bin.
    last_phase: Vec<f32>,
    /// Accumulated (per-bin) synthesis phase.
    sum_phase: Vec<f32>,
    /// Spectral-peak bins of the current frame (for identity phase
    /// locking), reused across frames.
    peaks: Vec<usize>,
    ola: Ola,
    /// Fractional start of the next analysis frame, relative to the input
    /// FIFO front. Fractional so the average analysis hop is exact (no
    /// per-frame rounding drift in the output length).
    cursor: f64,
}

impl PhaseVocoder {
    fn new() -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FRAME);
        let ifft = planner.plan_fft_inverse(FRAME);
        let scratch_len = fft
            .get_inplace_scratch_len()
            .max(ifft.get_inplace_scratch_len());
        Self {
            fft,
            ifft,
            window: hann_window(FRAME),
            spectrum: vec![Complex::new(0.0, 0.0); FRAME],
            scratch: vec![Complex::new(0.0, 0.0); scratch_len],
            frame_buf: vec![0.0; FRAME],
            mag: vec![0.0; FRAME / 2 + 1],
            anal_phase: vec![0.0; FRAME / 2 + 1],
            last_phase: vec![0.0; FRAME / 2 + 1],
            sum_phase: vec![0.0; FRAME / 2 + 1],
            peaks: Vec::new(),
            ola: Ola::new(),
            cursor: 0.0,
        }
    }

    fn reset(&mut self) {
        self.last_phase.iter_mut().for_each(|p| *p = 0.0);
        self.sum_phase.iter_mut().for_each(|p| *p = 0.0);
        self.peaks.clear();
        self.ola.reset();
        self.cursor = 0.0;
    }

    fn run(&mut self, input: &mut Fifo, out: &mut Fifo, factor: f32) {
        let factor = factor as f64;
        let analysis_hop = (SYNTH_HOP as f64 / factor).max(1.0);
        let synth_hop = SYNTH_HOP as f64;
        let bins = FRAME / 2 + 1;

        // Consume whole frames while the input FIFO holds one.
        loop {
            let start = self.cursor.round() as usize;
            if start + FRAME > input.len() {
                break;
            }

            // Windowed analysis frame → spectrum.
            for j in 0..FRAME {
                self.spectrum[j] = Complex::new(input.get(start + j) * self.window[j], 0.0);
            }
            self.fft
                .process_with_scratch(&mut self.spectrum, &mut self.scratch);

            // Pass 1: magnitude/phase + standard per-bin phase propagation
            // (each bin's instantaneous frequency drives its accumulator).
            for b in 0..bins {
                let re = self.spectrum[b].re;
                let im = self.spectrum[b].im;
                self.mag[b] = (re * re + im * im).sqrt();
                let phase = im.atan2(re);
                self.anal_phase[b] = phase;

                let omega = std::f32::consts::TAU * b as f32 / FRAME as f32;
                let expected = omega * analysis_hop as f32;
                let delta = princ_arg(phase - self.last_phase[b] - expected);
                let true_freq = omega + delta / analysis_hop as f32;
                self.last_phase[b] = phase;
                self.sum_phase[b] = princ_arg(self.sum_phase[b] + true_freq * synth_hop as f32);
            }

            // Pass 2: identity phase locking (Laroche & Dolson). Lock each
            // bin's synthesis phase to its nearest spectral peak, keeping
            // the within-frame phase *relationships* around every peak.
            // This preserves vertical coherence so the windowed sinusoids
            // reconstruct at full amplitude under non-unity stretch — the
            // "phase-locked" requirement; a plain per-bin vocoder loses
            // gain badly here.
            find_peaks(&self.mag, &mut self.peaks);
            if self.peaks.is_empty() {
                for b in 0..bins {
                    self.spectrum[b] = Complex::from_polar(self.mag[b], self.sum_phase[b]);
                }
            } else {
                let mut pk = 0usize;
                for b in 0..bins {
                    // Advance to the nearest peak at or after `b` if it is
                    // closer than the current one.
                    while pk + 1 < self.peaks.len()
                        && self.peaks[pk + 1].abs_diff(b) <= self.peaks[pk].abs_diff(b)
                    {
                        pk += 1;
                    }
                    let p = self.peaks[pk];
                    let locked =
                        princ_arg(self.sum_phase[p] + (self.anal_phase[b] - self.anal_phase[p]));
                    self.spectrum[b] = Complex::from_polar(self.mag[b], locked);
                }
            }
            // Hermitian-symmetric upper half for a real inverse transform.
            for b in 1..bins - 1 {
                self.spectrum[FRAME - b] = self.spectrum[b].conj();
            }

            self.ifft
                .process_with_scratch(&mut self.spectrum, &mut self.scratch);
            let norm = 1.0 / FRAME as f32;
            for j in 0..FRAME {
                self.frame_buf[j] = self.spectrum[j].re * norm;
            }

            let frame = std::mem::take(&mut self.frame_buf);
            self.ola.add_frame(&frame, &self.window, synth_hop);
            self.frame_buf = frame;

            self.cursor += analysis_hop;
        }

        // Drop the input the analysis window has fully passed (everything
        // before the next frame's start) and emit settled output: future
        // frames start at or after `ola.pos`, so lower samples are final.
        let consume = (self.cursor.floor() as usize).min(input.len());
        if consume > 0 {
            input.consume(consume);
            self.cursor -= consume as f64;
        }
        let settled = self.ola.pos.floor() as usize;
        self.ola.drain_below(settled, out);
    }
}

// ---------------------------------------------------------------------------
// WSOLA (Transient)
// ---------------------------------------------------------------------------

struct Wsola {
    window: Vec<f32>,
    ola: Ola,
    /// Fractional nominal analysis position into the input FIFO.
    analysis_pos: f64,
    /// The "natural continuation" the next frame should resemble (the
    /// overlap region that would follow the previous chosen frame at the
    /// unscaled rate). Empty until the first frame is placed.
    target: Vec<f32>,
    frame_buf: Vec<f32>,
}

impl Wsola {
    fn new() -> Self {
        Self {
            window: hann_window(FRAME),
            ola: Ola::new(),
            analysis_pos: 0.0,
            target: Vec::new(),
            frame_buf: vec![0.0; FRAME],
        }
    }

    fn reset(&mut self) {
        self.ola.reset();
        self.analysis_pos = 0.0;
        self.target.clear();
    }

    fn run(&mut self, input: &mut Fifo, out: &mut Fifo, factor: f32) {
        let factor = factor as f64;
        let analysis_hop = (SYNTH_HOP as f64 / factor).max(1.0);
        let overlap = FRAME - SYNTH_HOP;

        loop {
            let nominal = self.analysis_pos.round() as usize;
            // The first frame is placed at δ=0 (no continuation to match
            // yet) and needs only a full frame of input; later frames
            // search ±WSOLA_SEARCH and so need that much extra lookahead.
            let delta = if self.target.is_empty() {
                if nominal + FRAME > input.len() {
                    break;
                }
                0
            } else {
                if nominal < WSOLA_SEARCH || nominal + WSOLA_SEARCH + FRAME > input.len() {
                    break;
                }
                best_offset(input, nominal, overlap, &self.target)
            };

            let start = (nominal as isize + delta).max(0) as usize;
            if start + FRAME > input.len() {
                break;
            }

            for j in 0..FRAME {
                self.frame_buf[j] = input.get(start + j);
            }
            let frame = std::mem::take(&mut self.frame_buf);
            self.ola.add_frame(&frame, &self.window, SYNTH_HOP as f64);
            self.frame_buf = frame;

            // Natural continuation the next frame should resemble: what
            // follows the chosen frame after one synthesis hop. Always in
            // range given the loop's lookahead guard.
            let tgt_start = start + SYNTH_HOP;
            self.target.clear();
            for j in 0..overlap {
                let idx = tgt_start + j;
                self.target
                    .push(if idx < input.len() { input.get(idx) } else { 0.0 });
            }

            self.analysis_pos += analysis_hop;
        }

        // Drop input behind the search window, then emit settled output
        // (future frames start at or after `ola.pos`).
        let safe_consume = (self.analysis_pos.floor() as usize)
            .saturating_sub(WSOLA_SEARCH)
            .min(input.len());
        if safe_consume > 0 {
            input.consume(safe_consume);
            self.analysis_pos -= safe_consume as f64;
        }
        let settled = self.ola.pos.floor() as usize;
        self.ola.drain_below(settled, out);
    }
}

/// Find the offset `δ ∈ [-WSOLA_SEARCH, WSOLA_SEARCH]` that maximises the
/// normalised cross-correlation between `input[nominal+δ .. +overlap]` and
/// `target`. Returns `δ` (may be negative).
fn best_offset(input: &Fifo, nominal: usize, overlap: usize, target: &[f32]) -> isize {
    let len = overlap.min(target.len());
    if len == 0 {
        return 0;
    }
    let lo = -(WSOLA_SEARCH as isize).min(nominal as isize);
    let hi = WSOLA_SEARCH as isize;
    let mut best_delta = 0isize;
    let mut best_score = f32::NEG_INFINITY;
    let mut delta = lo;
    while delta <= hi {
        let start = nominal as isize + delta;
        if start < 0 || start as usize + len > input.len() {
            delta += 1;
            continue;
        }
        let start = start as usize;
        let mut dot = 0.0f32;
        let mut energy = 0.0f32;
        // Indexes both the FIFO (via `get`) and `target`; a zip would be
        // less clear than the shared index here.
        #[allow(clippy::needless_range_loop)]
        for j in 0..len {
            let s = input.get(start + j);
            dot += s * target[j];
            energy += s * s;
        }
        let score = dot / (energy.sqrt() + 1e-9);
        if score > best_score {
            best_score = score;
            best_delta = delta;
        }
        delta += 1;
    }
    best_delta
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Collect the spectral-peak bins of `mag` (local maxima over a ±2-bin
/// neighbourhood, above a tiny fraction of the frame's peak so noise-floor
/// ripple is ignored) into `peaks`, ascending. Used for identity phase
/// locking.
fn find_peaks(mag: &[f32], peaks: &mut Vec<usize>) {
    peaks.clear();
    let n = mag.len();
    if n == 0 {
        return;
    }
    let max = mag.iter().copied().fold(0.0f32, f32::max);
    let threshold = max * 1e-4;
    for b in 0..n {
        let m = mag[b];
        if m <= threshold {
            continue;
        }
        let lo = b.saturating_sub(2);
        let hi = (b + 2).min(n - 1);
        let is_peak = (lo..=hi).all(|k| k == b || mag[k] <= m);
        if is_peak {
            peaks.push(b);
        }
    }
}

/// Wrap a phase to the principal range (−π, π].
fn princ_arg(phase: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let mut p = phase;
    while p > PI {
        p -= TAU;
    }
    while p < -PI {
        p += TAU;
    }
    p
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
