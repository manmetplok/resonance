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

    /// This source converted to `rate` by the shared band-limited
    /// resampler (`resonance_common::resample_stereo`); itself when it is
    /// already there. The engine converts every source it publishes, off
    /// the audio thread, so the mixer only ever reads a cache frame for
    /// frame (code review FU-G3a). `cache_ref` still describes the file.
    pub fn at_rate(self, rate: u32) -> Self {
        if self.sample_rate == rate || rate == 0 || self.sample_rate == 0 {
            return self;
        }
        let samples = resonance_common::resample::resample_stereo(
            &self.samples,
            self.sample_rate as f32,
            rate as f32,
        );
        let frame_count = (samples.len() / 2) as u64;
        Self {
            cache_ref: self.cache_ref,
            samples: Arc::new(samples),
            sample_rate: rate,
            frame_count,
        }
    }
}
