//! Stereo bus-glue compressor.
//!
//! Classic feed-forward topology optimised for mastering-bus use:
//! max-of-channels peak detector, log-domain soft-knee gain computer,
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

/// When the disable-fade gain is within this of unity it snaps to 1.0
/// (≈0.001 dB — far below audibility) and the fade stops running.
const RESIDUAL_GAIN_EPS: f32 = 1e-4;

/// Streaming stereo glue compressor.
pub struct GlueCompressor {
    sample_rate: f32,
    /// Max-of-channels peak envelope (linear).
    peak_env: f32,
    /// Current gain reduction in dB (positive means attenuation).
    gr_db: f32,
    /// Meter decay for the reported GR readout.
    meter_gr_db: f32,
    meter_decay: f32,
    /// Disable-fade gain. Normally exactly 1.0; when the stage is
    /// switched off under gain reduction it starts at the last applied
    /// composite gain (GR + makeup, mix-blended) and relaxes to unity
    /// at the compressor's release rate, so toggling the stage doesn't
    /// step the level by the full GR in one sample.
    residual_gain: f32,
    /// Composite gain applied to the last sample of the most recent
    /// enabled block — the starting point for the disable fade.
    last_gain: f32,
    /// `enabled` of the previous block, to detect the disable edge.
    was_enabled: bool,
}

impl GlueCompressor {
    pub fn new(sample_rate: f32) -> Self {
        let mut c = Self {
            sample_rate,
            peak_env: 0.0,
            gr_db: 0.0,
            meter_gr_db: 0.0,
            meter_decay: 0.0,
            residual_gain: 1.0,
            last_gain: 1.0,
            was_enabled: false,
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
        self.residual_gain = 1.0;
        self.last_gain = 1.0;
        self.was_enabled = false;
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
            if self.was_enabled {
                // Disable edge: keep applying the last composite gain
                // and let it relax to unity below, instead of stepping
                // the level by the full GR (+ makeup, mix-blended) in
                // one sample.
                self.residual_gain = self.last_gain;
                self.was_enabled = false;
            }
            self.fade_out_disabled(left, right, cfg);
            return;
        }
        self.was_enabled = true;

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

            // Max-of-channels peak detector: fast attack, exponential
            // release. Level detection on a mastering bus must not
            // depend on stereo correlation: a mono-sum detector reads
            // anti-phase / side-dominant material near zero (a loud
            // wide band would get no gain reduction at all) and
            // hard-panned material 6 dB low. Tracking the louder
            // channel measures the actual level regardless of the
            // stereo image, and keeps the channels GR-linked.
            let abs_sample = l.abs().max(r.abs());
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
            let mut out_l = l + (wet_l - l) * mix;
            let mut out_r = r + (wet_r - r) * mix;

            // Finish any disable-fade remnant: re-enabling mid-fade
            // restarts the compressor from zero GR, so the fade keeps
            // relaxing to unity here to stay continuous. Exactly 1.0
            // (the steady state) skips the multiply entirely.
            if self.residual_gain != 1.0 {
                self.residual_gain = 1.0 + (self.residual_gain - 1.0) * release_coef;
                if (self.residual_gain - 1.0).abs() < RESIDUAL_GAIN_EPS {
                    self.residual_gain = 1.0;
                }
                out_l *= self.residual_gain;
                out_r *= self.residual_gain;
            }
            left[i] = out_l;
            right[i] = out_r;
        }

        // Remember the composite gain the block ended on, so a disable
        // edge can fade out from it instead of stepping to unity.
        self.last_gain =
            (1.0 + (db_to_linear(-self.gr_db) * makeup_lin - 1.0) * mix) * self.residual_gain;

        // Post-block meter: track the peak GR with a slow decay.
        self.meter_gr_db = if max_gr_block > self.meter_gr_db {
            max_gr_block
        } else {
            self.meter_gr_db * self.meter_decay
        };
    }

    /// Disabled-path fade: relax the residual composite gain to unity
    /// at the compressor's release rate. Ramping over the *release
    /// time* (rather than a fixed short ramp) is the musically sane
    /// choice — it is exactly the rate the user asked the compressor
    /// to let go at, so a toggle sounds like the compressor releasing,
    /// not like a fader move. Once the gain is within
    /// [`RESIDUAL_GAIN_EPS`] of unity this is a no-op and the disabled
    /// stage passes audio through untouched, bit-for-bit.
    fn fade_out_disabled(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        cfg: &GlueCompressorConfig,
    ) {
        if self.residual_gain == 1.0 {
            return;
        }
        let release_coef =
            Ballistics::from_times(self.sample_rate, cfg.attack_ms, cfg.release_ms).release_coef;
        let frames = left.len().min(right.len());
        for i in 0..frames {
            self.residual_gain = 1.0 + (self.residual_gain - 1.0) * release_coef;
            left[i] *= self.residual_gain;
            right[i] *= self.residual_gain;
        }
        if (self.residual_gain - 1.0).abs() < RESIDUAL_GAIN_EPS {
            self.residual_gain = 1.0;
        }
    }

    /// Current gain reduction meter value in dB (positive = reduction).
    pub fn meter_gr_db(&self) -> f32 {
        self.meter_gr_db
    }
}

