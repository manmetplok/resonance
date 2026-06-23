//! Freeze-related engine types.

use std::sync::Arc;

use resonance_common::FreezeCacheRef;

/// A decoded freeze-cache buffer attached to a track for playback.
///
/// When a track carries a `FrozenSource` the mixer can replay the cached
/// audio instead of running the live instrument + FX chain. The buffer is
/// timeline-aligned (rendered from sample 0 by
/// [`crate::engine::bounce::to_freeze_cache`]), so playback needs no stored
/// offset.
#[derive(Debug, Clone)]
pub struct FrozenSource {
    /// Metadata describing the on-disk freeze-cache file this was decoded
    /// from (filename, sample rate, bit depth, render fingerprint, status).
    pub cache_ref: FreezeCacheRef,
    /// The decoded audio samples, interleaved stereo L/R. Shared via `Arc`
    /// so the audio thread reads it without copying.
    pub samples: Arc<Vec<f32>>,
    /// Sample rate of the decoded audio.
    pub sample_rate: u32,
    /// Total number of stereo frames (`samples.len() / 2`).
    pub frame_count: u64,
}

impl FrozenSource {
    /// Build a frozen source from a cache reference and its decoded samples.
    pub fn new(
        cache_ref: FreezeCacheRef,
        samples: Arc<Vec<f32>>,
        sample_rate: u32,
        frame_count: u64,
    ) -> Self {
        Self {
            cache_ref,
            samples,
            sample_rate,
            frame_count,
        }
    }
}
