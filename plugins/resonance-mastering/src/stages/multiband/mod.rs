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

pub mod delay;
pub mod lowpass;

use std::sync::Arc;

use crate::stages::glue_compressor::{GlueCompressor, GlueCompressorConfig};
use crate::stages::linear_phase_eq::DesignWorker;
use delay::DelayLine;
use lowpass::LinearPhaseLowpass;
use resonance_dsp::db_to_linear;

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

    /// `cfg.enabled` of the previous chunk, used to detect the enable
    /// edge so the idled crossover filters can be restarted cleanly.
    was_enabled: bool,
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
            was_enabled: false,
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
        self.was_enabled = false;
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

        let just_enabled = cfg.enabled && !self.was_enabled;
        self.was_enabled = cfg.enabled;

        if !cfg.enabled {
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

        if just_enabled {
            // The crossovers idled during bypass, so their streaming
            // state is stale. Restart them from silence: the subtraction
            // topology sums the four bands to the delayed input for any
            // filter state, so the output stays continuous — the band
            // boundaries just settle over one FIR length instead of
            // leaking pre-bypass audio into the compressors.
            self.xo1.reset();
            self.xo2.reset();
            self.xo3.reset();
        }

        // Keep the crossover filters in sync with the current config.
        self.xo1.set_cutoff(cfg.crossover_hz[0]);
        self.xo2.set_cutoff(cfg.crossover_hz[1]);
        self.xo3.set_cutoff(cfg.crossover_hz[2]);

        self.run_crossover_network(left, right, frames);
        self.build_band_signals(frames);
        self.compress_bands(cfg, frames);
        self.sum_bands(cfg, left, right, frames);
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

    /// Stage 4: trim each band by its output gain and add the four band
    /// scratch buffers back into the caller's stereo buffers.
    ///
    /// The trim lives here rather than inside the band compressor so it
    /// is a property of the *band*, not of its compressor: it works with
    /// the compressor off, which is what makes the stage usable as a
    /// static four-band tone balancer. At the default 0 dB every gain is
    /// exactly 1.0 and the sum is unchanged.
    fn sum_bands(&self, cfg: &MultibandConfig, left: &mut [f32], right: &mut [f32], frames: usize) {
        let g = [
            band_gain(cfg.bands[0].gain_db),
            band_gain(cfg.bands[1].gain_db),
            band_gain(cfg.bands[2].gain_db),
            band_gain(cfg.bands[3].gain_db),
        ];
        for i in 0..frames {
            left[i] = self.y1_l[i] * g[0]
                + self.y2_l[i] * g[1]
                + self.y3_l[i] * g[2]
                + self.xd_l[i] * g[3];
            right[i] = self.y1_r[i] * g[0]
                + self.y2_r[i] * g[1]
                + self.y3_r[i] * g[2]
                + self.xd_r[i] * g[3];
        }
    }
}
