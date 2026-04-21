//! K-weighting filter as specified in ITU-R BS.1770-4.
//!
//! K-weighting is two cascaded biquads applied per channel before the
//! mean-square measurement used by the LUFS family of loudness metrics.
//!
//! 1. **Pre-filter**: a high-shelf boost approximating the head-related
//!    transfer function, centred at 1681.974450955533 Hz with a gain of
//!    ~3.999843853973347 dB and Q ~0.7071752369554196.
//! 2. **RLB filter**: a 2nd-order high-pass at 38.13547087602444 Hz,
//!    Q ~0.5003270373238773, rolling off sub-bass content that does not
//!    contribute meaningfully to perceived loudness.
//!
//! The coefficients are derived directly from the bilinear transform of
//! the analog prototypes so the filter is correct at any sample rate.
//! At 48 kHz the coefficients match the tabulated reference in BS.1770-4
//! Annex 1 Tables 1 and 2 (verified to 1e-5).
//!
//! Matches the reference implementations used by `libebur128` and
//! `pyloudnorm` — specifically, the RLB numerator is left as the literal
//! `[1, -2, 1]` derived from the bilinear transform (not further
//! normalized by the denominator `a0` factor), which mirrors those
//! references and the EBU test vector expectations.

use std::f32::consts::PI;

use resonance_dsp::Biquad;

/// Pre-filter centre frequency (Hz). BS.1770-4 Annex 1 Table 1.
const PREFILTER_F0: f32 = 1_681.974_4;
/// Pre-filter gain (dB).
const PREFILTER_GAIN_DB: f32 = 3.999_843_9;
/// Pre-filter Q.
const PREFILTER_Q: f32 = 0.707_175_24;
/// Exponent used to compute Vb from Vh in the shelf formula.
const VB_EXPONENT: f32 = 0.499_666_78;

/// RLB high-pass centre frequency (Hz). BS.1770-4 Annex 1 Table 2.
const RLB_F0: f32 = 38.135_47;
/// RLB high-pass Q.
const RLB_Q: f32 = 0.500_327_04;

/// Two cascaded biquads that apply the BS.1770-4 K-weighting curve to a
/// single audio channel. One instance per channel.
#[derive(Clone, Copy)]
pub struct KWeightingFilter {
    prefilter: Biquad,
    rlb: Biquad,
}

impl KWeightingFilter {
    /// Create a K-weighting filter configured for `sample_rate` Hz.
    pub fn new(sample_rate: f32) -> Self {
        let mut f = Self {
            prefilter: Biquad::identity(),
            rlb: Biquad::identity(),
        };
        f.set_sample_rate(sample_rate);
        f
    }

    /// Recompute coefficients for a new sample rate. Does not clear state.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        assign_prefilter(&mut self.prefilter, sample_rate);
        assign_rlb(&mut self.rlb, sample_rate);
    }

    /// Clear the filter's internal delay lines without touching coefficients.
    pub fn reset(&mut self) {
        self.prefilter.reset();
        self.rlb.reset();
    }

    /// Filter one sample and return the K-weighted value.
    #[inline]
    pub fn process(&mut self, sample: f32) -> f32 {
        let y1 = self.prefilter.process(sample);
        self.rlb.process(y1)
    }
}

/// Compute and assign the high-shelf pre-filter coefficients.
/// Matches the derivation used in pyloudnorm / libebur128.
fn assign_prefilter(bq: &mut Biquad, sr: f32) {
    let k = (PI * PREFILTER_F0 / sr).tan();
    let vh = 10.0_f32.powf(PREFILTER_GAIN_DB / 20.0);
    let vb = vh.powf(VB_EXPONENT);
    let k_sq = k * k;
    let k_over_q = k / PREFILTER_Q;

    let a0 = 1.0 + k_over_q + k_sq;
    let inv_a0 = 1.0 / a0;

    bq.b0 = (vh + vb * k_over_q + k_sq) * inv_a0;
    bq.b1 = 2.0 * (k_sq - vh) * inv_a0;
    bq.b2 = (vh - vb * k_over_q + k_sq) * inv_a0;
    bq.a1 = 2.0 * (k_sq - 1.0) * inv_a0;
    bq.a2 = (1.0 - k_over_q + k_sq) * inv_a0;
}

/// Compute and assign the RLB high-pass coefficients.
/// Uses the literal `b = [1, -2, 1]` numerator form, mirroring the
/// libebur128 and pyloudnorm reference implementations.
fn assign_rlb(bq: &mut Biquad, sr: f32) {
    let k = (PI * RLB_F0 / sr).tan();
    let k_sq = k * k;
    let k_over_q = k / RLB_Q;

    let a0 = 1.0 + k_over_q + k_sq;
    let inv_a0 = 1.0 / a0;

    bq.b0 = 1.0;
    bq.b1 = -2.0;
    bq.b2 = 1.0;
    bq.a1 = 2.0 * (k_sq - 1.0) * inv_a0;
    bq.a2 = (1.0 - k_over_q + k_sq) * inv_a0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefilter_matches_bs1770_48k_reference() {
        let mut bq = Biquad::identity();
        assign_prefilter(&mut bq, 48_000.0);
        // BS.1770-4 Annex 1 Table 1 values, tolerance 1e-4.
        assert!((bq.b0 - 1.535_124_9).abs() < 1e-4, "b0 = {}", bq.b0);
        assert!((bq.b1 - -2.691_696_2).abs() < 1e-4, "b1 = {}", bq.b1);
        assert!((bq.b2 - 1.198_392_8).abs() < 1e-4, "b2 = {}", bq.b2);
        assert!((bq.a1 - -1.690_659_3).abs() < 1e-4, "a1 = {}", bq.a1);
        assert!((bq.a2 - 0.732_480_8).abs() < 1e-4, "a2 = {}", bq.a2);
    }

    #[test]
    fn rlb_matches_bs1770_48k_reference() {
        let mut bq = Biquad::identity();
        assign_rlb(&mut bq, 48_000.0);
        assert!((bq.b0 - 1.0).abs() < 1e-6);
        assert!((bq.b1 - -2.0).abs() < 1e-6);
        assert!((bq.b2 - 1.0).abs() < 1e-6);
        assert!((bq.a1 - -1.990_047_5).abs() < 1e-4, "a1 = {}", bq.a1);
        assert!((bq.a2 - 0.990_072_2).abs() < 1e-4, "a2 = {}", bq.a2);
    }

    #[test]
    fn processes_without_nans() {
        let mut f = KWeightingFilter::new(48_000.0);
        for i in 0..4096 {
            let t = i as f32 / 48_000.0;
            let x = (t * 1000.0 * std::f32::consts::TAU).sin() * 0.5;
            let y = f.process(x);
            assert!(y.is_finite());
        }
    }

    #[test]
    fn passes_through_mid_band_near_unity() {
        // The K-weighting curve is gentle across the midrange. Verified
        // values at 48 kHz (from both this implementation and reference
        // plots of BS.1770-4 K-weighting):
        //   1 kHz ≈ +0.6 dB, 2 kHz ≈ +1.0 dB, 5 kHz ≈ +3.0 dB.
        // The 1 kHz gain must be within a narrow band centred on +0.6.
        let sr = 48_000.0;
        let mut f = KWeightingFilter::new(sr);
        for _ in 0..4096 {
            let _ = f.process(0.0);
        }
        let freq = 1000.0_f32;
        let n = 48_000usize;
        let mut in_sq = 0.0_f64;
        let mut out_sq = 0.0_f64;
        for i in 0..n {
            let x = (i as f32 / sr * freq * std::f32::consts::TAU).sin();
            let y = f.process(x);
            if i > 4096 {
                in_sq += (x * x) as f64;
                out_sq += (y * y) as f64;
            }
        }
        let gain_db = 10.0 * (out_sq / in_sq).log10();
        assert!(
            (gain_db - 0.6).abs() < 0.3,
            "K-weighted 1 kHz gain = {gain_db} dB (expected ≈ +0.6)"
        );
    }

    #[test]
    fn attenuates_low_frequencies() {
        // Below 100 Hz the RLB high-pass dominates and the signal is
        // attenuated by many dB. A 30 Hz sine should drop by at least 6 dB.
        let sr = 48_000.0;
        let mut f = KWeightingFilter::new(sr);
        for _ in 0..4096 {
            let _ = f.process(0.0);
        }
        let freq = 30.0_f32;
        let n = 48_000usize;
        let mut in_sq = 0.0_f64;
        let mut out_sq = 0.0_f64;
        for i in 0..n {
            let x = (i as f32 / sr * freq * std::f32::consts::TAU).sin();
            let y = f.process(x);
            if i > 4096 {
                in_sq += (x * x) as f64;
                out_sq += (y * y) as f64;
            }
        }
        let gain_db = 10.0 * (out_sq / in_sq).log10();
        assert!(
            gain_db < -6.0,
            "30 Hz K-weighted gain = {gain_db} dB (expected < -6)"
        );
    }
}
