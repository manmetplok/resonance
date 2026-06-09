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
#[doc(hidden)]
pub fn assign_prefilter(bq: &mut Biquad, sr: f32) {
    let k = (PI * PREFILTER_F0 / sr).tan();
    let vh = 10.0_f32.powf(PREFILTER_GAIN_DB / 20.0);
    let vb = vh.powf(VB_EXPONENT);
    let k_sq = k * k;
    let k_over_q = k / PREFILTER_Q;

    let a0 = 1.0 + k_over_q + k_sq;
    let inv_a0 = 1.0 / a0;

    bq.assign_raw(
        (vh + vb * k_over_q + k_sq) * inv_a0,
        2.0 * (k_sq - vh) * inv_a0,
        (vh - vb * k_over_q + k_sq) * inv_a0,
        2.0 * (k_sq - 1.0) * inv_a0,
        (1.0 - k_over_q + k_sq) * inv_a0,
    );
}

/// Compute and assign the RLB high-pass coefficients.
/// Uses the literal `b = [1, -2, 1]` numerator form, mirroring the
/// libebur128 and pyloudnorm reference implementations.
#[doc(hidden)]
pub fn assign_rlb(bq: &mut Biquad, sr: f32) {
    let k = (PI * RLB_F0 / sr).tan();
    let k_sq = k * k;
    let k_over_q = k / RLB_Q;

    let a0 = 1.0 + k_over_q + k_sq;
    let inv_a0 = 1.0 / a0;

    bq.assign_raw(
        1.0,
        -2.0,
        1.0,
        2.0 * (k_sq - 1.0) * inv_a0,
        (1.0 - k_over_q + k_sq) * inv_a0,
    );
}

