//! Stereo bus-glue compressor.
//!
//! Classic feed-forward topology optimised for mastering-bus use:
//! mono-sum peak detector, log-domain soft-knee gain computer,
//! attack/release ballistics on the gain-reduction envelope, parallel
//! mix for transparent blending. Defaults are slow-attack / slow-release
//! so drum transients pass through and the compressor only levels the
//! sustained energy.
//!
//! The math is a trimmed version of `resonance-compressor` (Bob Katz's
//! log-domain formulation). No RMS blend, no sidechain HPF — those are
//! adequate in a dedicated track compressor but are not what you want
//! on a mastering bus where the detector must stay honest.

use resonance_dsp::{db_to_linear, linear_to_db, soft_knee_gain_reduction_db, Ballistics};

/// Plain-data snapshot of the compressor's current parameter values.
/// Built once per audio block from the atomic plugin params.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlueCompressorConfig {
    pub enabled: bool,
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub knee_db: f32,
    pub makeup_db: f32,
    /// Parallel mix — 1.0 = fully compressed, 0.0 = dry.
    pub mix: f32,
}

impl Default for GlueCompressorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_db: -18.0,
            ratio: 2.0,
            attack_ms: 30.0,
            release_ms: 150.0,
            knee_db: 6.0,
            makeup_db: 0.0,
            mix: 1.0,
        }
    }
}

/// Streaming stereo glue compressor.
pub struct GlueCompressor {
    sample_rate: f32,
    /// Mono-sum peak envelope (linear).
    peak_env: f32,
    /// Current gain reduction in dB (positive means attenuation).
    gr_db: f32,
    /// Meter decay for the reported GR readout.
    meter_gr_db: f32,
    meter_decay: f32,
}

impl GlueCompressor {
    pub fn new(sample_rate: f32) -> Self {
        let mut c = Self {
            sample_rate,
            peak_env: 0.0,
            gr_db: 0.0,
            meter_gr_db: 0.0,
            meter_decay: 0.0,
        };
        c.set_sample_rate(sample_rate);
        c
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
        // GR meter decays ~250 ms visually.
        self.meter_decay = (-1.0_f32 / (0.25 * sr)).exp();
    }

    pub fn reset(&mut self) {
        self.peak_env = 0.0;
        self.gr_db = 0.0;
        self.meter_gr_db = 0.0;
    }

    /// Process a stereo block in place. Leaves audio unchanged if the
    /// compressor is disabled.
    pub fn process_stereo(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        cfg: &GlueCompressorConfig,
    ) {
        if !cfg.enabled {
            // Drain internal state and let the GR meter decay so the
            // UI falls back to 0 dB promptly and re-enabling the stage
            // starts from a clean slate.
            self.peak_env = 0.0;
            self.gr_db = 0.0;
            self.meter_gr_db *= self.meter_decay;
            return;
        }

        let ballistics = Ballistics::from_times(self.sample_rate, cfg.attack_ms, cfg.release_ms);
        let release_coef = ballistics.release_coef;
        let knee = cfg.knee_db.max(0.0);
        let half_knee = knee * 0.5;
        let ratio = cfg.ratio.max(1.0);
        let slope = 1.0 - 1.0 / ratio;
        let makeup_lin = db_to_linear(cfg.makeup_db);
        let mix = cfg.mix.clamp(0.0, 1.0);
        let threshold = cfg.threshold_db;

        let frames = left.len().min(right.len());
        let mut max_gr_block = self.meter_gr_db;

        for i in 0..frames {
            let l = left[i];
            let r = right[i];

            // Mono-sum peak detector: fast attack, exponential release.
            let mono = 0.5 * (l + r);
            let abs_sample = mono.abs();
            self.peak_env = if abs_sample > self.peak_env {
                abs_sample
            } else {
                abs_sample + (self.peak_env - abs_sample) * release_coef
            };

            // Static soft-knee gain computer + attack/release ballistics.
            let detector_db = linear_to_db(self.peak_env);
            let target_gr_db =
                soft_knee_gain_reduction_db(detector_db, threshold, knee, half_knee, slope);
            self.gr_db = ballistics.step_envelope(self.gr_db, target_gr_db);

            if self.gr_db > max_gr_block {
                max_gr_block = self.gr_db;
            }

            // Apply gain reduction + makeup, blend parallel.
            let apply_lin = db_to_linear(-self.gr_db) * makeup_lin;
            let wet_l = l * apply_lin;
            let wet_r = r * apply_lin;
            left[i] = l + (wet_l - l) * mix;
            right[i] = r + (wet_r - r) * mix;
        }

        // Post-block meter: track the peak GR with a slow decay.
        self.meter_gr_db = if max_gr_block > self.meter_gr_db {
            max_gr_block
        } else {
            self.meter_gr_db * self.meter_decay
        };
    }

    /// Current gain reduction meter value in dB (positive = reduction).
    pub fn meter_gr_db(&self) -> f32 {
        self.meter_gr_db
    }
}

