//! Linear-phase multiband compressor.
//!
//! Splits the stereo signal into four bands using three cascaded linear-
//! phase lowpass filters (the LR-style "subtraction" crossover network),
//! compresses each band independently via a [`GlueCompressor`], and
//! sums the results. The four bands sum to the delayed input when all
//! compression is disabled (perfect reconstruction).
//!
//! Because every lowpass shares the same group delay and every band is
//! aligned to that delay, the crossover does not introduce phase
//! distortion between bands — the multiband is truly "transparent" when
//! the per-band compressors are bypassed.
//!
//! # Per-band width
//!
//! The imager's per-band width (`img_b{n}_width`, warmth-width-depth.md
//! §6.3) reuses this crossover instead of building a second one: each
//! band's side channel is scaled by its width just before the bands are
//! summed. The imager itself is linear and runs right after, so scaling
//! here is the same as scaling there, and it costs no extra latency.
//! While any width is off unity the crossover runs even with the
//! multiband disabled (the compressors and band trims stay out of it,
//! as they are while disabled). At unity everywhere nothing of this
//! runs, and the stage is exactly what it was without it.
//!
//! # Switching on and off (DSP2-05)
//!
//! Enabling and disabling crossfade over [`ENABLE_RAMP_MS`] between the
//! processed bands and the plain split (whose sum is the delayed input),
//! and band trims ramp over [`GAIN_RAMP_MS`]. When the crossover restarts
//! after idling, its filters refill from silence for one FIR window,
//! during which `band_3 = delayed_input − y3` carries nearly the whole
//! mix; the stage then stays on the plain split (no compression, trims or
//! width) until the bands are real, and only then fades the processing
//! in. A freshly built or reset stage starts in its configured state with
//! no ramp: its delay line restarted along with the filters, so the bands
//! are consistent from the first sample.

pub mod delay;
pub mod lowpass;

use std::sync::Arc;

use crate::stages::glue_compressor::{GlueCompressor, GlueCompressorConfig};
use crate::stages::linear_phase_eq::{DesignWorker, FirGeometry};
use delay::DelayLine;
use lowpass::LinearPhaseLowpass;
use resonance_dsp::db_to_linear;
use resonance_plugin::{Smoother, SmoothingStyle};

use super::retarget;

/// Ramp length of the per-band width smoothers, in milliseconds.
const WIDTH_RAMP_MS: f32 = 10.0;
/// Crossfade length of the enable / disable switch, in milliseconds.
pub const ENABLE_RAMP_MS: f32 = 10.0;
/// Ramp length of the per-band output trims, in milliseconds.
pub const GAIN_RAMP_MS: f32 = 10.0;

/// Number of frequency bands.
pub const NUM_BANDS: usize = 4;

/// Plain-data snapshot of every multiband parameter.
#[derive(Debug, Clone, Copy)]
pub struct MultibandConfig {
    pub enabled: bool,
    /// Three crossover frequencies, low-to-high. Must be monotonic.
    pub crossover_hz: [f32; 3],
    /// Per-band compressor settings.
    pub bands: [BandConfig; NUM_BANDS],
}

/// Per-band settings as exposed by the plugin.
///
/// Every band runs a full [`GlueCompressor`], so it takes the same seven
/// controls the single-band glue stage does — the defining multiband
/// move (fast release on the lows, slow on the highs) is exactly the
/// per-band `release_ms` here.
///
/// `gain_db` is the odd one out: it is a **band output trim**, not the
/// compressor's makeup, so it applies whether or not that band's
/// compressor is enabled and the multiband can be used as a static
/// four-band tone balancer. (With `mix < 1.0` it therefore trims the
/// blended band, dry part included — which is what a band output level
/// should do.)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandConfig {
    pub enabled: bool,
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub knee_db: f32,
    /// Parallel mix — 1.0 = fully compressed, 0.0 = dry.
    pub mix: f32,
    pub gain_db: f32,
}

impl Default for BandConfig {
    fn default() -> Self {
        // Attack / release / knee / mix match the glue stage's defaults,
        // which is what the band compressors were hardcoded to before
        // they became parameters — an untouched project sounds the same.
        Self {
            enabled: false,
            threshold_db: -18.0,
            ratio: 2.0,
            attack_ms: 30.0,
            release_ms: 150.0,
            knee_db: 6.0,
            mix: 1.0,
            gain_db: 0.0,
        }
    }
}

impl Default for MultibandConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            crossover_hz: [120.0, 800.0, 4000.0],
            bands: [BandConfig::default(); NUM_BANDS],
        }
    }
}

/// Linear gain for a band trim.
///
/// 0 dB returns exactly `1.0f32`, so a band left at the default is
/// multiplied by a true identity and its contribution to the sum is
/// bit-for-bit what it was before the trim existed. (`db_to_linear(0.0)`
/// happens to be exactly 1.0 too; the special case makes that a
/// guarantee of this function rather than a property of another one.)
pub fn band_gain(gain_db: f32) -> f32 {
    if gain_db == 0.0 {
        1.0
    } else {
        db_to_linear(gain_db)
    }
}

/// Streaming four-band compressor with linear-phase crossovers.
pub struct Multiband {
    max_buffer: usize,

    xo1: LinearPhaseLowpass,
    xo2: LinearPhaseLowpass,
    xo3: LinearPhaseLowpass,

    /// Per-band compressors (each handles stereo internally).
    band_comps: [GlueCompressor; NUM_BANDS],

    /// Sample delay on the input path so `band_3 = delayed_input − y3`
    /// can be computed at the same time offset as the lowpass outputs.
    delay_left: DelayLine,
    delay_right: DelayLine,

    /// Scratch buffers for the three lowpass outputs (stereo).
    y1_l: Vec<f32>,
    y1_r: Vec<f32>,
    y2_l: Vec<f32>,
    y2_r: Vec<f32>,
    y3_l: Vec<f32>,
    y3_r: Vec<f32>,
    /// Scratch buffers for the delayed input path (stereo).
    xd_l: Vec<f32>,
    xd_r: Vec<f32>,

    /// Whether the previous chunk ran the crossover network, used to
    /// detect the edge where it restarts so the idled filters can be
    /// restarted cleanly.
    was_splitting: bool,

    /// Per-band side gain (the imager's per-band width), smoothed per
    /// sample. `width_tgt` mirrors the last requested target.
    width_sm: [Smoother; NUM_BANDS],
    width_tgt: [f32; NUM_BANDS],

    /// Amount of processing (compression + trims) mixed over the plain
    /// split: 0 = off, 1 = on. Ramped on every enable / disable.
    wet_sm: Smoother,
    wet_tgt: f32,
    /// Per-band output trims (linear), ramped.
    gain_sm: [Smoother; NUM_BANDS],
    gain_tgt: [f32; NUM_BANDS],
    /// Samples the crossover still needs to refill after a restart.
    settle_left: usize,
    /// One crossover window: how long a restart takes to refill.
    settle_len: usize,
    /// False until the first chunk after construction / [`Self::reset`],
    /// which snaps to the configured state instead of ramping.
    primed: bool,
    /// Uncompressed copies of the four bands, for the enable crossfade.
    dry_l: [Vec<f32>; NUM_BANDS],
    dry_r: [Vec<f32>; NUM_BANDS],
    /// Per-sample wet amount for the current chunk while it ramps.
    wet_buf: Vec<f32>,
}

impl Multiband {
    /// A multiband whose crossovers design on their own worker thread.
    pub fn new(sample_rate: f32, max_buffer: usize) -> Self {
        Self::with_worker(sample_rate, max_buffer, Some(&DesignWorker::spawn()))
    }

    /// A multiband whose crossovers design through `worker` (or always
    /// inline with `None`; the output is identical either way).
    pub fn with_worker(sample_rate: f32, max_buffer: usize, worker: Option<&Arc<DesignWorker>>) -> Self {
        let default = MultibandConfig::default();
        let xo = |hz| LinearPhaseLowpass::with_worker(sample_rate, hz, worker);
        let xo1 = xo(default.crossover_hz[0]);
        let delay_len = xo1.latency();
        Self {
            max_buffer,
            xo1,
            xo2: xo(default.crossover_hz[1]),
            xo3: xo(default.crossover_hz[2]),
            band_comps: [
                GlueCompressor::new(sample_rate),
                GlueCompressor::new(sample_rate),
                GlueCompressor::new(sample_rate),
                GlueCompressor::new(sample_rate),
            ],
            delay_left: DelayLine::new(delay_len),
            delay_right: DelayLine::new(delay_len),
            y1_l: vec![0.0; max_buffer],
            y1_r: vec![0.0; max_buffer],
            y2_l: vec![0.0; max_buffer],
            y2_r: vec![0.0; max_buffer],
            y3_l: vec![0.0; max_buffer],
            y3_r: vec![0.0; max_buffer],
            xd_l: vec![0.0; max_buffer],
            xd_r: vec![0.0; max_buffer],
            was_splitting: false,
            width_sm: std::array::from_fn(|_| {
                let mut sm = Smoother::new(SmoothingStyle::Linear(WIDTH_RAMP_MS));
                sm.set_sample_rate(sample_rate);
                sm.reset(1.0);
                sm
            }),
            width_tgt: [1.0; NUM_BANDS],
            wet_sm: ramp(sample_rate, ENABLE_RAMP_MS, 0.0),
            wet_tgt: 0.0,
            gain_sm: std::array::from_fn(|_| ramp(sample_rate, GAIN_RAMP_MS, 1.0)),
            gain_tgt: [1.0; NUM_BANDS],
            settle_left: 0,
            settle_len: {
                let g = FirGeometry::for_sample_rate(sample_rate);
                g.latency() + g.group_delay
            },
            primed: false,
            dry_l: std::array::from_fn(|_| vec![0.0; max_buffer]),
            dry_r: std::array::from_fn(|_| vec![0.0; max_buffer]),
            wet_buf: vec![0.0; max_buffer],
        }
    }

    pub fn reset(&mut self) {
        self.xo1.reset();
        self.xo2.reset();
        self.xo3.reset();
        for c in self.band_comps.iter_mut() {
            c.reset();
        }
        self.delay_left.reset();
        self.delay_right.reset();
        self.was_splitting = false;
        for (sm, tgt) in self.width_sm.iter_mut().zip(self.width_tgt.iter_mut()) {
            sm.reset(1.0);
            *tgt = 1.0;
        }
        self.wet_sm.reset(0.0);
        self.wet_tgt = 0.0;
        for sm in self.gain_sm.iter_mut() {
            sm.reset(1.0);
        }
        self.gain_tgt = [1.0; NUM_BANDS];
        self.settle_left = 0;
        self.primed = false;
    }

    /// Stage latency in samples (identical to one linear-phase lowpass;
    /// scales with the sample rate).
    pub fn latency(&self) -> usize {
        self.xo1.latency()
    }

    /// [`Self::latency`] of a multiband built for `sample_rate`.
    pub fn latency_for(sample_rate: f32) -> usize {
        LinearPhaseLowpass::latency_for(sample_rate)
    }

    /// Stagger the three crossovers' FFT iterations, per crossover and
    /// channel (see [`LinearPhaseLowpass::set_phase_offsets`]).
    pub fn set_phase_offsets(&mut self, offsets: [[usize; 2]; 3]) {
        self.xo1.set_phase_offsets(offsets[0]);
        self.xo2.set_phase_offsets(offsets[1]);
        self.xo3.set_phase_offsets(offsets[2]);
    }

    /// Samples until each crossover channel's next FFT iteration.
    pub fn iteration_countdowns(&self) -> [[usize; 2]; 3] {
        [
            self.xo1.iteration_countdowns(),
            self.xo2.iteration_countdowns(),
            self.xo3.iteration_countdowns(),
        ]
    }

    /// Crossover designs taken from the worker vs. designed inline,
    /// summed over the three lowpasses (diagnostics).
    pub fn design_counts(&self) -> (u64, u64) {
        [&self.xo1, &self.xo2, &self.xo3]
            .iter()
            .map(|x| x.design_counts())
            .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
    }

    /// Current gain reduction of each band's compressor in dB (positive
    /// = attenuation), low band first.
    ///
    /// Free: every band compressor already tracks this for its own
    /// meter, decayed over ~250 ms so the value is readable rather than
    /// flickering. Reading it adds nothing to the audio thread.
    pub fn band_gr_db(&self) -> [f32; NUM_BANDS] {
        std::array::from_fn(|i| self.band_comps[i].meter_gr_db())
    }

    /// Process a stereo block in place.
    ///
    /// Scratch is sized for `max_buffer` frames, which the plugin's
    /// `initialize` provides from the host's declared maximum block
    /// size (the whole chain is rebuilt there, off the audio thread).
    /// If a host nevertheless delivers a larger block, it is processed
    /// in `max_buffer`-sized chunks — the filters, delay lines, and
    /// compressors are all streaming, so chunking is transparent and
    /// no frame is ever silently dropped. No allocation either way.
    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32], cfg: &MultibandConfig) {
        self.process_stereo_with_width(left, right, cfg, &[1.0; NUM_BANDS]);
    }

    /// [`Self::process_stereo`] with a per-band side gain (the imager's
    /// per-band width, already `1.0` everywhere when the imager is off):
    /// see the module docs. Values are clamped to `0..=2` and ramped.
    pub fn process_stereo_with_width(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        cfg: &MultibandConfig,
        width: &[f32; NUM_BANDS],
    ) {
        for ((sm, tgt), w) in self.width_sm.iter_mut().zip(&mut self.width_tgt).zip(width) {
            retarget(sm, tgt, w.clamp(0.0, 2.0));
        }
        let total = left.len().min(right.len());
        let mut start = 0;
        while start < total {
            let frames = (total - start).min(self.max_buffer);
            self.process_chunk(
                &mut left[start..start + frames],
                &mut right[start..start + frames],
                cfg,
            );
            start += frames;
        }
    }

    /// Process one chunk of at most `max_buffer` frames in place.
    fn process_chunk(&mut self, left: &mut [f32], right: &mut [f32], cfg: &MultibandConfig) {
        let frames = left.len().min(right.len());
        debug_assert!(frames <= self.max_buffer);
        if frames == 0 {
            return;
        }

        let first = !self.primed;
        self.primed = true;
        if first {
            // A fresh or reset stage starts as configured, no ramps.
            self.wet_tgt = if cfg.enabled { 1.0 } else { 0.0 };
            self.wet_sm.reset(self.wet_tgt);
            for b in 0..NUM_BANDS {
                self.gain_tgt[b] = band_gain(cfg.bands[b].gain_db);
                self.gain_sm[b].reset(self.gain_tgt[b]);
            }
        }

        let widening = self.width_active();
        // A disable keeps the split running until its fade-out is done.
        let processing = cfg.enabled || self.wet_sm.current() != 0.0;
        let splitting = processing || widening;
        let just_split = splitting && !self.was_splitting;
        self.was_splitting = splitting;

        if !splitting {
            // Bypass path: output = delayed input. The crossovers' only
            // contribution here would be their group delay, which the
            // input delay line reproduces exactly, so skip the three FIR
            // convolutions and run only the delay. Latency stays at
            // `latency()` either way, which the chain's latency model
            // and whole-plugin bypass delay depend on.
            for i in 0..frames {
                left[i] = self.delay_left.push(left[i]);
                right[i] = self.delay_right.push(right[i]);
            }
            return;
        }

        if just_split {
            // The crossovers idled during bypass, so their streaming
            // state is stale. Restart them from silence: the subtraction
            // topology sums the four bands to the delayed input for any
            // filter state, so the output stays continuous. The band
            // boundaries are wrong until the filters refill, though, so
            // hold the processing off until then (the delay line kept
            // running, so this is not needed on the first chunk, where
            // both start from silence together).
            self.xo1.reset();
            self.xo2.reset();
            self.xo3.reset();
            if !first {
                self.settle_left = self.settle_len;
            }
        }
        let settling = self.settle_left > 0;
        self.settle_left = self.settle_left.saturating_sub(frames);

        // Keep the crossover filters in sync with the current config.
        self.xo1.set_cutoff(cfg.crossover_hz[0]);
        self.xo2.set_cutoff(cfg.crossover_hz[1]);
        self.xo3.set_cutoff(cfg.crossover_hz[2]);

        self.run_crossover_network(left, right, frames);
        self.build_band_signals(frames);

        let wet_target = if cfg.enabled && !settling { 1.0 } else { 0.0 };
        if wet_target == 1.0 && self.wet_sm.current() == 0.0 {
            // Fading in from fully off: the band compressors last ran
            // who knows when (they are skipped while off), so start
            // them clean, exactly like a fresh stage.
            for c in self.band_comps.iter_mut() {
                c.reset();
            }
        }
        retarget(&mut self.wet_sm, &mut self.wet_tgt, wet_target);
        for b in 0..NUM_BANDS {
            let g = band_gain(cfg.bands[b].gain_db);
            retarget(&mut self.gain_sm[b], &mut self.gain_tgt[b], g);
        }
        self.process_bands(cfg, frames);
        if widening && !settling {
            self.widen_bands(frames);
        }
        self.sum_bands(left, right, frames);
    }

    /// Compress and trim the four bands, crossfaded against the plain
    /// split by the wet amount. Fully off, the bands are left as split
    /// (bit-for-bit what a disabled stage always produced); fully on,
    /// each band is compressed and trimmed in place, exactly as before
    /// the crossfade existed.
    fn process_bands(&mut self, cfg: &MultibandConfig, frames: usize) {
        let ramping = self.wet_sm.current() != self.wet_tgt;
        if !ramping && self.wet_tgt == 0.0 {
            // Off: the trims are inaudible, so let them land.
            for sm in self.gain_sm.iter_mut() {
                sm.skip(frames as u32);
            }
            return;
        }
        if ramping {
            for w in self.wet_buf[..frames].iter_mut() {
                *w = self.wet_sm.next();
            }
            let Self {
                y1_l,
                y1_r,
                y2_l,
                y2_r,
                y3_l,
                y3_r,
                xd_l,
                xd_r,
                dry_l,
                dry_r,
                ..
            } = self;
            let bands: [(&[f32], &[f32]); NUM_BANDS] =
                [(y1_l, y1_r), (y2_l, y2_r), (y3_l, y3_r), (xd_l, xd_r)];
            for (b, (l, r)) in bands.into_iter().enumerate() {
                dry_l[b][..frames].copy_from_slice(&l[..frames]);
                dry_r[b][..frames].copy_from_slice(&r[..frames]);
            }
        }
        self.compress_bands(cfg, frames);

        let Self {
            y1_l,
            y1_r,
            y2_l,
            y2_r,
            y3_l,
            y3_r,
            xd_l,
            xd_r,
            dry_l,
            dry_r,
            gain_sm,
            gain_tgt,
            wet_buf,
            ..
        } = self;
        let bands: [(&mut [f32], &mut [f32]); NUM_BANDS] =
            [(y1_l, y1_r), (y2_l, y2_r), (y3_l, y3_r), (xd_l, xd_r)];
        for (b, (l, r)) in bands.into_iter().enumerate() {
            let (l, r) = (&mut l[..frames], &mut r[..frames]);
            let sm = &mut gain_sm[b];
            if ramping {
                let (dl, dr) = (&dry_l[b][..frames], &dry_r[b][..frames]);
                for i in 0..frames {
                    let g = sm.next();
                    let w = wet_buf[i];
                    l[i] = dl[i] + (l[i] * g - dl[i]) * w;
                    r[i] = dr[i] + (r[i] * g - dr[i]) * w;
                }
            } else if gain_tgt[b] != 1.0 || sm.current() != 1.0 {
                for i in 0..frames {
                    let g = sm.next();
                    l[i] *= g;
                    r[i] *= g;
                }
            }
        }
    }

    /// True while any band's width is off unity or still ramping.
    fn width_active(&self) -> bool {
        self.width_sm
            .iter()
            .zip(&self.width_tgt)
            .any(|(sm, &t)| t != 1.0 || sm.current() != 1.0)
    }

    /// Scale each band's side channel by its (smoothed) width, in place.
    /// A band at rest at unity is left untouched.
    fn widen_bands(&mut self, frames: usize) {
        let bands: [(&mut [f32], &mut [f32]); NUM_BANDS] = [
            (&mut self.y1_l[..frames], &mut self.y1_r[..frames]),
            (&mut self.y2_l[..frames], &mut self.y2_r[..frames]),
            (&mut self.y3_l[..frames], &mut self.y3_r[..frames]),
            (&mut self.xd_l[..frames], &mut self.xd_r[..frames]),
        ];
        for (b, (l, r)) in bands.into_iter().enumerate() {
            let sm = &mut self.width_sm[b];
            if self.width_tgt[b] == 1.0 && sm.current() == 1.0 {
                continue;
            }
            for i in 0..frames {
                let w = sm.next();
                let mid = 0.5 * (l[i] + r[i]);
                let side = 0.5 * (l[i] - r[i]) * w;
                l[i] = mid + side;
                r[i] = mid - side;
            }
        }
    }

    /// Stage 1: route raw input through the delay line into `xd_*`, and
    /// convolve three copies of the input through the lowpass cascade
    /// into `y1_*`, `y2_*`, `y3_*`. After this step, every scratch
    /// buffer corresponds to the *same* input time — the FIR group
    /// delay and the delay line are identical.
    fn run_crossover_network(&mut self, left: &[f32], right: &[f32], frames: usize) {
        for i in 0..frames {
            self.xd_l[i] = self.delay_left.push(left[i]);
            self.xd_r[i] = self.delay_right.push(right[i]);
        }

        self.y1_l[..frames].copy_from_slice(&left[..frames]);
        self.y1_r[..frames].copy_from_slice(&right[..frames]);
        self.xo1
            .process_stereo(&mut self.y1_l[..frames], &mut self.y1_r[..frames]);

        self.y2_l[..frames].copy_from_slice(&left[..frames]);
        self.y2_r[..frames].copy_from_slice(&right[..frames]);
        self.xo2
            .process_stereo(&mut self.y2_l[..frames], &mut self.y2_r[..frames]);

        self.y3_l[..frames].copy_from_slice(&left[..frames]);
        self.y3_r[..frames].copy_from_slice(&right[..frames]);
        self.xo3
            .process_stereo(&mut self.y3_l[..frames], &mut self.y3_r[..frames]);
    }

    /// Stage 2: subtract the cascaded lowpass outputs from one another
    /// to form four disjoint bands, reusing the `y*` / `xd_*` buffers
    /// in place:
    ///   band_0 (sub) = y1
    ///   band_1 (low-mid) = y2 − y1
    ///   band_2 (high-mid) = y3 − y2
    ///   band_3 (air) = delayed_input − y3
    fn build_band_signals(&mut self, frames: usize) {
        for i in 0..frames {
            let b1_l = self.y2_l[i] - self.y1_l[i];
            let b1_r = self.y2_r[i] - self.y1_r[i];
            let b2_l = self.y3_l[i] - self.y2_l[i];
            let b2_r = self.y3_r[i] - self.y2_r[i];
            let b3_l = self.xd_l[i] - self.y3_l[i];
            let b3_r = self.xd_r[i] - self.y3_r[i];
            // band_0 already lives in y1_*; leave it in place.
            self.y2_l[i] = b1_l;
            self.y2_r[i] = b1_r;
            self.y3_l[i] = b2_l;
            self.y3_r[i] = b2_r;
            // band_3 replaces the delayed-input scratch (no longer needed).
            self.xd_l[i] = b3_l;
            self.xd_r[i] = b3_r;
        }
    }

    /// Stage 3: run each band's scratch buffer through its dedicated
    /// glue compressor. Config comes straight from the plugin params.
    ///
    /// The band's `gain_db` is deliberately *not* passed as the
    /// compressor's makeup: makeup only reaches the wet path, and the
    /// compressor returns early when it is disabled, which used to make
    /// the band Gain control silent on a band whose compressor was off.
    /// It is applied to the band output in [`Self::sum_bands`] instead.
    fn compress_bands(&mut self, cfg: &MultibandConfig, frames: usize) {
        let band_lefts: [&mut [f32]; NUM_BANDS] = [
            &mut self.y1_l[..frames],
            &mut self.y2_l[..frames],
            &mut self.y3_l[..frames],
            &mut self.xd_l[..frames],
        ];
        let band_rights: [&mut [f32]; NUM_BANDS] = [
            &mut self.y1_r[..frames],
            &mut self.y2_r[..frames],
            &mut self.y3_r[..frames],
            &mut self.xd_r[..frames],
        ];

        let mut band_lefts = band_lefts.into_iter();
        let mut band_rights = band_rights.into_iter();
        for (comp, band) in self.band_comps.iter_mut().zip(cfg.bands.iter()) {
            let sub_cfg = GlueCompressorConfig {
                enabled: band.enabled,
                threshold_db: band.threshold_db,
                ratio: band.ratio,
                attack_ms: band.attack_ms,
                release_ms: band.release_ms,
                knee_db: band.knee_db,
                makeup_db: 0.0,
                mix: band.mix,
            };
            let l = band_lefts.next().unwrap();
            let r = band_rights.next().unwrap();
            comp.process_stereo(l, r, &sub_cfg);
        }
    }

    /// Stage 4: add the four (processed) band buffers back into the
    /// caller's stereo buffers.
    ///
    /// Band trims were applied in [`Self::process_bands`], so they are a
    /// property of the *band*, not of its compressor: they work with the
    /// compressor off, which is what makes the stage usable as a static
    /// four-band tone balancer.
    fn sum_bands(&self, left: &mut [f32], right: &mut [f32], frames: usize) {
        for i in 0..frames {
            left[i] = self.y1_l[i] + self.y2_l[i] + self.y3_l[i] + self.xd_l[i];
            right[i] = self.y1_r[i] + self.y2_r[i] + self.y3_r[i] + self.xd_r[i];
        }
    }
}

/// A linear smoother at `value`, ramping over `ms`.
fn ramp(sample_rate: f32, ms: f32, value: f32) -> Smoother {
    let mut sm = Smoother::new(SmoothingStyle::Linear(ms));
    sm.set_sample_rate(sample_rate);
    sm.reset(value);
    sm
}
