//! Core compressor DSP: detector → static gain computer → ballistics →
//! apply → makeup → mix.
//!
//! Topology is a classic log-domain feed-forward compressor. Mono sum of
//! the stereo input optionally runs through a sidechain high-pass, then
//! feeds both a fast peak envelope and a 30 ms RMS envelope. A detector
//! blend parameter crossfades between the two in dB space and hands the
//! result to a static soft-knee gain computer that returns a target GR
//! in dB. That target is smoothed with separate attack and release
//! one-pole coefficients and applied to the stereo signal as a gain,
//! after which manual and optional auto makeup gain are added and the
//! parallel mix control blends between the dry input and the compressed
//! path.
//!
//! All intermediate quantities downstream of the detector are in dB so
//! that the soft-knee formula and makeup gain are linear and cheap.

use resonance_dsp::{db_to_linear, linear_to_db, soft_knee_gain_reduction_db, Ballistics, Biquad};

use crate::params::CompressorParams;
use crate::viz::{CompressorViz, HISTORY_STEP_SAMPLES};

pub struct CompressorDsp {
    sample_rate: f32,

    /// Peak envelope of the detector signal (linear magnitude).
    peak_env: f32,
    /// RMS envelope of the detector signal (mean-square, linear).
    rms_env: f32,
    /// One-pole coefficient for the RMS smoother. Independent of attack
    /// because the RMS smoother is a signal-smoother, not a gain smoother.
    rms_coef: f32,

    /// Current gain reduction in dB after attack/release smoothing.
    gr_db: f32,

    /// Sidechain high-pass biquad, applied to the mono detector signal.
    sc_hpf: Biquad,

    /// Accumulator that decides when to push a GR sample into the viz ring.
    history_accum: u32,

    /// Running peak meters (linear) for the input and output, smoothed so
    /// the meters don't flicker.
    in_peak: f32,
    out_peak: f32,
    meter_decay: f32,
}

impl CompressorDsp {
    pub fn new(sample_rate: f32) -> Self {
        let mut dsp = Self {
            sample_rate,
            peak_env: 0.0,
            rms_env: 0.0,
            rms_coef: 0.0,
            gr_db: 0.0,
            sc_hpf: Biquad::identity(),
            history_accum: 0,
            in_peak: 0.0,
            out_peak: 0.0,
            meter_decay: 0.0,
        };
        dsp.set_sample_rate(sample_rate);
        dsp
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
        // RMS smoother time constant: ~30 ms window.
        self.rms_coef = (-1.0_f32 / (0.030 * sr)).exp();
        // Meter decay: ~250 ms to drop ~60 dB visually.
        self.meter_decay = (-1.0_f32 / (0.25 * sr)).exp();
    }

    pub fn reset(&mut self) {
        self.peak_env = 0.0;
        self.rms_env = 0.0;
        self.gr_db = 0.0;
        self.sc_hpf.reset();
        self.history_accum = 0;
        self.in_peak = 0.0;
        self.out_peak = 0.0;
    }

    /// Process a stereo block in place. All parameter reads happen at the
    /// top of the call so the detector and gain-computer math stay hot
    /// inside the per-sample loop.
    pub fn process_stereo(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        params: &CompressorParams,
        viz: &CompressorViz,
    ) {
        let frames = left.len().min(right.len());
        if frames == 0 {
            return;
        }

        // --- Snapshot parameters for this block ---
        let threshold = params.threshold.value();
        let ratio = params.ratio.value().max(1.0);
        let knee = params.knee.value().max(0.0);
        let attack_ms = params.attack.value().max(0.05);
        let release_ms = params.release.value().max(1.0);
        let detector_mix = params.detector_mix.value().clamp(0.0, 1.0);
        let makeup_manual_db = params.makeup.value();
        let mix_target = params.mix.value().clamp(0.0, 1.0);
        let auto_makeup = params.auto_makeup.value();
        let sc_hpf_on = params.sc_hpf_on.value();
        let sc_hpf_freq = params.sc_hpf_freq.value();

        // --- Derived quantities ---
        // Attack/release coefficients: one-pole exponential convergence.
        // `exp(-1 / (time_seconds * sr))` is the fraction kept each sample.
        let ballistics = Ballistics::from_times(self.sample_rate, attack_ms, release_ms);
        let release_coef = ballistics.release_coef;

        // Auto-makeup: compensate about half the maximum possible GR at
        // 0 dBFS input, which is a good perceptual match for music that
        // rarely hits the full dBFS ceiling.
        let auto_gain_db = if auto_makeup {
            -threshold * (1.0 - 1.0 / ratio) * 0.5
        } else {
            0.0
        };
        let total_makeup_db = makeup_manual_db + auto_gain_db;

        // Update SC HPF coefficients once per block. When the HPF is
        // disabled we bypass by using an identity biquad (same coefficient
        // path, effectively a no-op).
        if sc_hpf_on {
            self.sc_hpf
                .set_high_pass(self.sample_rate, sc_hpf_freq, 0.707);
        } else {
            self.sc_hpf.set_identity();
        }

        let half_knee = knee * 0.5;
        let slope = 1.0 - 1.0 / ratio;

        // Per-sample loop.
        let mut in_peak_block: f32 = self.in_peak;
        let mut out_peak_block: f32 = self.out_peak;

        for i in 0..frames {
            let l = left[i];
            let r = right[i];

            // Detection signal: mono sum routed through the optional
            // sidechain HPF. HPF is biquad; an identity biquad returns
            // the sample unchanged with a tiny state cost.
            let mono = 0.5 * (l + r);
            let det_sample = self.sc_hpf.process(mono);

            // Peak envelope: fast attack, exponential decay. The release
            // coefficient is also used for the peak decay here so the
            // detector respects the user's release time.
            let abs_sample = det_sample.abs();
            self.peak_env = if abs_sample > self.peak_env {
                abs_sample
            } else {
                abs_sample + (self.peak_env - abs_sample) * release_coef
            };

            // RMS envelope: 30 ms mean-square smoother.
            let sq = det_sample * det_sample;
            self.rms_env = sq + (self.rms_env - sq) * self.rms_coef;

            // Convert to dB and blend.
            let peak_db = linear_to_db(self.peak_env);
            let rms_db = linear_to_db(self.rms_env.sqrt());
            let detector_db = peak_db * (1.0 - detector_mix) + rms_db * detector_mix;

            // Static knee/ratio nonlinearity.
            let target_gr_db =
                soft_knee_gain_reduction_db(detector_db, threshold, knee, half_knee, slope);

            // Attack/release ballistics on the GR envelope. When new GR is
            // larger than current (the comp needs to clamp harder) we use
            // the attack coefficient; otherwise the slower release.
            self.gr_db = ballistics.step_envelope(self.gr_db, target_gr_db);

            // Apply the gain reduction plus makeup.
            let apply_db = total_makeup_db - self.gr_db;
            let apply_lin = db_to_linear(apply_db);

            let wet_l = l * apply_lin;
            let wet_r = r * apply_lin;

            // Parallel mix.
            let out_l = l * (1.0 - mix_target) + wet_l * mix_target;
            let out_r = r * (1.0 - mix_target) + wet_r * mix_target;

            left[i] = out_l;
            right[i] = out_r;

            // Meter envelopes (slow decay, instant attack).
            let abs_in = abs_sample.max(l.abs()).max(r.abs());
            in_peak_block = if abs_in > in_peak_block {
                abs_in
            } else {
                in_peak_block * self.meter_decay
            };
            let abs_out = out_l.abs().max(out_r.abs());
            out_peak_block = if abs_out > out_peak_block {
                abs_out
            } else {
                out_peak_block * self.meter_decay
            };

            // GR history ring: push the current GR once every
            // HISTORY_STEP_SAMPLES samples.
            self.history_accum += 1;
            if self.history_accum >= HISTORY_STEP_SAMPLES {
                self.history_accum = 0;
                viz.push_gr(self.gr_db);
            }
        }

        self.in_peak = in_peak_block;
        self.out_peak = out_peak_block;

        // Publish the latest scalar meter values once per block.
        viz.store_levels(
            linear_to_db(in_peak_block),
            linear_to_db(out_peak_block),
            self.gr_db,
        );
    }
}

/// Public, pure helper reused by the editor to render the transfer curve
/// without instantiating a whole DSP.
pub fn transfer_curve_db(
    input_db: f32,
    threshold: f32,
    ratio: f32,
    knee: f32,
    makeup_db: f32,
) -> f32 {
    let ratio = ratio.max(1.0);
    let slope = 1.0 - 1.0 / ratio;
    let half_knee = knee * 0.5;
    let gr = soft_knee_gain_reduction_db(input_db, threshold, knee, half_knee, slope);
    input_db - gr + makeup_db
}
