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

use crate::engine::audition::{audition_gen, AUDITION_PLAYING};
use crate::engine::SharedState;

/// Mix the active audition preview (if any) into `data` in place.
///
/// Advances the audition playhead by the published ratio per output frame,
/// wrapping at the source end when looping or latching `audition_finished`
/// (and stopping) on a non-looping run that reaches the end. A no-op when no
/// preview is playing.
pub fn mix_audition_overlay(data: &mut [f32], channels: usize, shared: &SharedState) {
    // Acquire pairs with the Release in `start_audition_in_place`: a
    // playing word observed here guarantees the source / start / ratio /
    // loop stores sequenced before it are visible.
    let ctl = shared.audition_ctl.load(Ordering::Acquire);
    if ctl & AUDITION_PLAYING == 0 {
        return;
    }
    let run = audition_gen(ctl);
    let guard = shared.audition_source.load();
    let Some(source) = guard.as_ref() else {
        return;
    };
    let frame_count = source.frame_count as usize;
    if frame_count == 0 {
        // Degenerate empty source: report finished so the engine thread
        // emits AuditionStopped, and stop.
        latch_finish(shared, ctl);
        return;
    }

    let samples = source.samples.as_slice();
    let fc = frame_count as f64;
    let looping = shared.audition_loop.load(Ordering::Relaxed);
    let ratio = f32::from_bits(shared.audition_ratio_bits.load(Ordering::Relaxed)).max(0.0) as f64;
    // Continue from the position this run last reached; a run this
    // callback has not advanced yet starts where the engine asked (code
    // review RT-11: the old unconditional load/store pair carried a
    // previous run's position over a restart).
    let mut pos = if shared.audition_pos_gen.load(Ordering::Relaxed) == run {
        f64::from_bits(shared.audition_pos_bits.load(Ordering::Relaxed))
    } else {
        f64::from_bits(shared.audition_start_bits.load(Ordering::Relaxed))
    };

    let out_frames = data.len() / channels;
    let mut finished = false;
    for f in 0..out_frames {
        if pos >= fc {
            if looping {
                // pos % fc, robust to a ratio that overshot by >1 loop.
                pos -= fc * (pos / fc).floor();
                if pos >= fc {
                    pos -= fc;
                }
            } else {
                finished = true;
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

    // Sole writer of the position's generation; a restart that landed
    // during this block bumped the run, so the next block ignores this
    // position and starts the new run from its own start.
    shared
        .audition_pos_bits
        .store(pos.to_bits(), Ordering::Relaxed);
    shared.audition_pos_gen.store(run, Ordering::Relaxed);
    if finished {
        latch_finish(shared, ctl);
    }
}

/// End the run this block played (`ctl`), unless the engine restarted or
/// stopped it meanwhile: the compare-exchange fails then, leaving the new
/// run playing (RT-11).
fn latch_finish(shared: &SharedState, ctl: u64) {
    if shared
        .audition_ctl
        .compare_exchange(ctl, ctl & !AUDITION_PLAYING, Ordering::AcqRel, Ordering::Relaxed)
        .is_ok()
    {
        shared
            .audition_finished
            .store(audition_gen(ctl) + 1, Ordering::Release);
    }
}
