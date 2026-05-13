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
#[cfg(test)]
mod tests;

use crate::stages::glue_compressor::{GlueCompressor, GlueCompressorConfig};
use delay::DelayLine;
use lowpass::LinearPhaseLowpass;

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

/// Per-band compressor settings as exposed by the plugin.
#[derive(Debug, Clone, Copy)]
pub struct BandConfig {
    pub enabled: bool,
    pub threshold_db: f32,
    pub ratio: f32,
    pub gain_db: f32,
}

impl Default for BandConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_db: -18.0,
            ratio: 2.0,
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
}

impl Multiband {
    pub fn new(sample_rate: f32, max_buffer: usize) -> Self {
        let default = MultibandConfig::default();
        let delay_len = LinearPhaseLowpass::latency();
        Self {
            max_buffer,
            xo1: LinearPhaseLowpass::new(sample_rate, default.crossover_hz[0]),
            xo2: LinearPhaseLowpass::new(sample_rate, default.crossover_hz[1]),
            xo3: LinearPhaseLowpass::new(sample_rate, default.crossover_hz[2]),
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
    }

    /// Stage latency in samples (identical to one linear-phase lowpass).
    pub const fn latency() -> usize {
        LinearPhaseLowpass::latency()
    }

    /// Process a stereo block in place.
    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32], cfg: &MultibandConfig) {
        let frames = left.len().min(right.len()).min(self.max_buffer);
        if frames == 0 {
            return;
        }

        // Keep the crossover filters in sync with the current config.
        self.xo1.set_cutoff(cfg.crossover_hz[0]);
        self.xo2.set_cutoff(cfg.crossover_hz[1]);
        self.xo3.set_cutoff(cfg.crossover_hz[2]);

        self.run_crossover_network(left, right, frames);

        if !cfg.enabled {
            // Bypass path: output = delayed input. Preserves latency so
            // the host doesn't see a latency change when toggling.
            left[..frames].copy_from_slice(&self.xd_l[..frames]);
            right[..frames].copy_from_slice(&self.xd_r[..frames]);
            return;
        }

        self.build_band_signals(frames);
        self.compress_bands(cfg, frames);
        self.sum_bands(left, right, frames);
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
                attack_ms: 30.0,
                release_ms: 150.0,
                knee_db: 6.0,
                makeup_db: band.gain_db,
                mix: 1.0,
            };
            let l = band_lefts.next().unwrap();
            let r = band_rights.next().unwrap();
            comp.process_stereo(l, r, &sub_cfg);
        }
    }

    /// Stage 4: add the four band scratch buffers back into the
    /// caller's stereo buffers.
    fn sum_bands(&self, left: &mut [f32], right: &mut [f32], frames: usize) {
        for i in 0..frames {
            left[i] = self.y1_l[i] + self.y2_l[i] + self.y3_l[i] + self.xd_l[i];
            right[i] = self.y1_r[i] + self.y2_r[i] + self.y3_r[i] + self.xd_r[i];
        }
    }
}
