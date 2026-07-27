//! Audition preview overlay, mixed on the cpal audio callback thread.
//!
//! The preview is summed into the output buffer *after* the arrangement and
//! the master pass, independent of transport state — so a sample audition is
//! audible whether or not the project is rolling, and at a level independent
//! of the master fader (it is a monitor-style preview, not part of the mix).
//!
//! Allocation-free: the decoded source is loaded wait-free from
//! [`SharedState::audition_source`](crate::engine::SharedState) and read in
//! place with linear interpolation between source frames, so a non-unit
//! playback ratio (sync-to-tempo varispeed, see [`crate::engine::audition`])
//! resamples on the fly without a scratch buffer.

use std::sync::atomic::Ordering;

use crate::engine::SharedState;

/// Mix the active audition preview (if any) into `data` in place.
///
/// Advances the audition playhead by the published ratio per output frame,
/// wrapping at the source end when looping or latching `audition_finished`
/// (and stopping) on a non-looping run that reaches the end. A no-op when no
/// preview is playing.
pub fn mix_audition_overlay(data: &mut [f32], channels: usize, shared: &SharedState) {
    if !shared.audition_playing.load(Ordering::Relaxed) {
        return;
    }
    let guard = shared.audition_source.load();
    let Some(source) = guard.as_ref() else {
        return;
    };
    let frame_count = source.frame_count as usize;
    if frame_count == 0 {
        // Degenerate empty source: report finished so the engine thread
        // emits AuditionStopped, and stop.
        shared.audition_playing.store(false, Ordering::Relaxed);
        shared.audition_finished.store(true, Ordering::Relaxed);
        return;
    }

    let samples = source.samples.as_slice();
    let fc = frame_count as f64;
    let looping = shared.audition_loop.load(Ordering::Relaxed);
    let ratio = f32::from_bits(shared.audition_ratio_bits.load(Ordering::Relaxed)).max(0.0) as f64;
    let mut pos = f64::from_bits(shared.audition_pos_bits.load(Ordering::Relaxed));

    let out_frames = data.len() / channels;
    for f in 0..out_frames {
        if pos >= fc {
            if looping {
                // pos % fc, robust to a ratio that overshot by >1 loop.
                pos -= fc * (pos / fc).floor();
                if pos >= fc {
                    pos -= fc;
                }
            } else {
                shared.audition_playing.store(false, Ordering::Relaxed);
                shared.audition_finished.store(true, Ordering::Relaxed);
                break;
            }
        }

        let i0 = pos.floor() as usize;
        let frac = (pos - i0 as f64) as f32;
        // Next frame for interpolation: wrap to 0 when looping, else hold the
        // last frame so the final sample doesn't read out of bounds.
        let i1 = if i0 + 1 < frame_count {
            i0 + 1
        } else if looping {
            0
        } else {
            i0
        };
        let l = samples[i0 * 2] + (samples[i1 * 2] - samples[i0 * 2]) * frac;
        let r = samples[i0 * 2 + 1] + (samples[i1 * 2 + 1] - samples[i0 * 2 + 1]) * frac;

        let base = f * channels;
        // Sum onto L/R and clamp: the preview bypasses the master hard-clip,
        // so guard the output against a runaway sum here.
        data[base] = (data[base] + l).clamp(-1.0, 1.0);
        if channels > 1 {
            data[base + 1] = (data[base + 1] + r).clamp(-1.0, 1.0);
        }

        pos += ratio;
    }

    shared
        .audition_pos_bits
        .store(pos.to_bits(), Ordering::Relaxed);
}
