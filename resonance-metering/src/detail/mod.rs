//! Opt-in detailed mix analysis (warmth-width-depth.md §2, §7.1).
//!
//! The `meter.measure` / `meter.stems` `detail` option: the measurable
//! proxies for warmth ([`spectrum`]), computed over a whole rendered
//! buffer. Pure functions
//! like [`offline`][crate::offline]: slice in, value out, no state kept,
//! allocates freely — never call them from the audio thread.
//!
//! Every detail reads one shared analysis: [`analyze_detail`] runs the
//! Welch auto/cross spectra ([`analyze_stereo`]) once, so asking for
//! several details costs one pass.
//!
//! [`analyze_stereo`]: crate::spectrum::offline::analyze_stereo

pub mod spectrum;

pub use spectrum::{spectrum_detail, SpectralPeak, SpectrumDetail, THIRD_OCTAVE_BANDS};

use crate::spectrum::offline::{analyze_stereo, StereoSpectrum, DETAIL_FFT_SIZE};

/// The shared spectral analysis every detail reads from.
pub fn analyze_detail(sample_rate: f32, left: &[f32], right: &[f32]) -> StereoSpectrum {
    analyze_stereo(sample_rate, left, right, DETAIL_FFT_SIZE)
}

/// Power ratio in dB, `None` when either side is not a positive, finite
/// power (silence carries no ratio).
pub(crate) fn ratio_db(num: f64, den: f64) -> Option<f32> {
    (num > 0.0 && den > 0.0 && num.is_finite() && den.is_finite())
        .then(|| (10.0 * (num / den).log10()) as f32)
}
