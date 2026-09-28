//! A reference track as the decision engine uses it: a name and the
//! analysis of its audio.
//!
//! The reference's LTAS becomes the target spectral shape and its
//! integrated LUFS the target loudness (see [`crate::decide::Target`]).
//! Getting the audio is the caller's business: the plugin decodes a file
//! the user picks, the app measures a pooled asset through its engine.

use crate::analyze::{self, AnalysisResult};

#[derive(Debug, Clone)]
pub struct ReferenceTrack {
    pub display_name: String,
    pub sample_rate: f32,
    pub analysis: AnalysisResult,
}

impl ReferenceTrack {
    /// A reference named `display_name`, analysed from its decoded
    /// stereo audio.
    pub fn from_samples(
        display_name: String,
        sample_rate: f32,
        left: &[f32],
        right: &[f32],
    ) -> Self {
        Self {
            display_name,
            sample_rate,
            analysis: analyze::run(sample_rate, left, right),
        }
    }
}
