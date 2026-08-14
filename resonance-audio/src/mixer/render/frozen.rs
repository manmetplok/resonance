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
/// Sample-rate mismatch between the cache and the engine is handled by
/// linear interpolation: with matching rates (the normal case — the
/// cache is rendered at the project rate) the read is frame-for-frame and
/// therefore bit-exact, which is what makes a frozen bounce sample-
/// identical to the unfrozen one. Returns whether any non-zero sample was
/// written. Allocation-free and `O(frames)`.
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

    if source.sample_rate == engine_sample_rate || engine_sample_rate == 0 {
        // Frame-for-frame copy — bit-exact, the parity-critical path.
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
    } else {
        // Rate mismatch: resample the cache on the fly by linear
        // interpolation. Timeline frame `tl` maps to cache position
        // `tl * cache_rate / engine_rate`.
        let ratio = source.sample_rate as f64 / engine_sample_rate as f64;
        for f in 0..frames {
            let tl = playhead + f as u64;
            let src_pos = tl as f64 * ratio;
            let i0 = src_pos.floor() as u64;
            if i0 >= cache_frames {
                break;
            }
            let frac = (src_pos - i0 as f64) as f32;
            let idx0 = i0 as usize * 2;
            if idx0 + 1 >= samples.len() {
                break;
            }
            let (l0, r0) = (samples[idx0], samples[idx0 + 1]);
            let (l1, r1) = if i0 + 1 < cache_frames && idx0 + 3 < samples.len() {
                (samples[idx0 + 2], samples[idx0 + 3])
            } else {
                (l0, r0)
            };
            let l = l0 + (l1 - l0) * frac;
            let r = r0 + (r1 - r0) * frac;
            track_buf_l[f] = l;
            track_buf_r[f] = r;
            has_audio |= l != 0.0 || r != 0.0;
        }
    }

    has_audio
}
