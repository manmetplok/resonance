//! Thin delegation layer over the consolidated decode/resample
//! implementation in `resonance_common` (`wav.rs`). Kept so
//! `crate::decode::*` call sites and the public
//! `resonance_audio::{linear_resample, StreamingLinearResampler}`
//! re-exports don't churn.

pub use resonance_common::{decode_file, StreamingLinearResampler};

/// Linear interpolation resampler for stereo interleaved audio with
/// `u32` rates. Delegates to [`resonance_common::linear_resample_stereo`].
pub fn linear_resample(input: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    resonance_common::linear_resample_stereo(input, source_rate as f32, target_rate as f32)
}
