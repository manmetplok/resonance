//! Frozen-source playback substitution: a track carrying an active
//! freeze cache reads it instead of running its instrument + insert FX.

use crate::types::FrozenSource;

/// Fill the de-interleaved track buffers from a track's [`FrozenSource`]
/// cache for the timeline window `[playhead, playhead + frames)`,
/// replacing the live instrument + insert-FX render (doc #187, todo
/// #573). The cache is interleaved stereo L/R, rendered from sample 0 so
/// timeline frame `t` maps directly to cache frame `t`; frames past the
/// end of the cache stay silent (the caller zeroed the buffers).
///
/// The read is frame-for-frame and therefore bit-exact, which is what
/// makes a frozen bounce sample-identical to the unfrozen one. A cache at
/// another rate never gets here: the engine converts it with the shared
/// resampler when it is published (`FrozenSource::at_rate`, code review
/// FU-G3a); one that somehow does plays silent rather than at the wrong
/// speed. Returns whether any non-zero sample was written.
/// Allocation-free and `O(frames)`.
pub(crate) fn fill_from_frozen_source(
    source: &FrozenSource,
    engine_sample_rate: u32,
    playhead: u64,
    frames: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
) -> bool {
    let samples = source.samples.as_slice();
    let cache_frames = source.frame_count;
    let mut has_audio = false;

    if source.sample_rate != engine_sample_rate && engine_sample_rate != 0 {
        return false;
    }
    for f in 0..frames {
        let tl = playhead + f as u64;
        if tl >= cache_frames {
            break;
        }
        let idx = tl as usize * 2;
        if idx + 1 >= samples.len() {
            break;
        }
        let (l, r) = (samples[idx], samples[idx + 1]);
        track_buf_l[f] = l;
        track_buf_r[f] = r;
        has_audio |= l != 0.0 || r != 0.0;
    }

    has_audio
}
