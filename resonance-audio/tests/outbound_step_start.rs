//! Unit tests for `outbound_step_start`, the helper that decides
//! whether an engine-thread MIDI poll should treat the playhead jump
//! as a normal step, a loop wrap (rewind to `loop_in`), or a genuine
//! seek/scrub (skip retroactive notes).
//!
//! The bug this guards against: on the second iteration of a loop,
//! the audio callback snaps `playhead` from `loop_out` back to
//! `loop_in` and then advances. By the time the engine thread polls
//! (~16 ms later), `curr` already sits past `loop_in`. Without the
//! rewind, `last` jumps to `curr` and the first note of the new
//! iteration falls outside `[last, curr)` and is silently dropped.

use resonance_audio::{outbound_step_start, OutboundStep};

const SR: u32 = 48_000;

#[test]
fn normal_forward_step_returns_continue() {
    // Playhead advanced by ~16 ms (one engine tick at 60 Hz).
    let step = outbound_step_start(10_000, 10_768, SR, false, 0, 0);
    assert_eq!(step, OutboundStep::Continue(10_000));
}

#[test]
fn normal_forward_step_at_zero_returns_continue() {
    // First poll after Play: last and curr are both at the seek
    // target. The note at `last` has to be eligible — half-open
    // `[last, curr)` only emits something when curr > last, so the
    // initial 0→0 case is handled by an early-return on equality
    // upstream. This helper just classifies the discontinuity.
    let step = outbound_step_start(0, 0, SR, false, 0, 0);
    assert_eq!(step, OutboundStep::Continue(0));
}

#[test]
fn loop_wrap_rewinds_to_loop_in() {
    // Loop = [1000, 5000). After the audio thread wrapped, the
    // playhead lives at 1100 (advanced 100 frames since `loop_in`).
    // The engine's previous `last_raw` was 4900 (mid-pre-wrap).
    let step = outbound_step_start(4_900, 1_100, SR, true, 1_000, 5_000);
    assert_eq!(step, OutboundStep::Discontinuity(Some(1_000)));
}

#[test]
fn loop_wrap_with_curr_exactly_at_loop_in() {
    // Race-rare edge case: engine polls between the audio thread
    // snapping the playhead and rendering the post-wrap sub-block,
    // so `curr == loop_in`. The rewind still applies; the upstream
    // `curr == last` check handles the no-op.
    let step = outbound_step_start(4_900, 1_000, SR, true, 1_000, 5_000);
    assert_eq!(step, OutboundStep::Discontinuity(Some(1_000)));
}

#[test]
fn backward_jump_without_loop_is_seek() {
    // User scrubbed backward with looping disabled — no notes
    // should fire retroactively.
    let step = outbound_step_start(10_000, 5_000, SR, false, 0, 0);
    assert_eq!(step, OutboundStep::Discontinuity(None));
}

#[test]
fn backward_jump_landing_outside_loop_is_seek() {
    // Loop is [1000, 5000) but the user scrubbed to 500 (before
    // loop_in). Treat as a genuine seek, not a loop wrap.
    let step = outbound_step_start(10_000, 500, SR, true, 1_000, 5_000);
    assert_eq!(step, OutboundStep::Discontinuity(None));
}

#[test]
fn backward_jump_landing_at_loop_out_is_seek() {
    // Loop is [1000, 5000). `curr == loop_out` is past the
    // half-open end of the loop, so it can't be a wrap landing.
    let step = outbound_step_start(10_000, 5_000, SR, true, 1_000, 5_000);
    assert_eq!(step, OutboundStep::Discontinuity(None));
}

#[test]
fn forward_jump_over_one_second_is_seek() {
    // Big forward jump (>1 s at 48 kHz): user seeked forward with
    // the timeline ruler, not a normal poll-to-poll advance.
    let step = outbound_step_start(10_000, 10_000 + SR as u64 + 1, SR, false, 0, 0);
    assert_eq!(step, OutboundStep::Discontinuity(None));
}

#[test]
fn loop_disabled_backward_jump_into_loop_range_is_seek() {
    // Loop range exists in the project but looping is off — the
    // backward jump can't be a wrap, so treat it as a seek.
    let step = outbound_step_start(10_000, 1_100, SR, false, 1_000, 5_000);
    assert_eq!(step, OutboundStep::Discontinuity(None));
}

#[test]
fn degenerate_loop_range_treated_as_seek() {
    // loop_out <= loop_in is a malformed loop range that the audio
    // path also rejects (`hi > lo` guard in mixer.rs). Skip the
    // rewind so we don't divide by zero or rewind to a bogus point.
    let step = outbound_step_start(10_000, 1_100, SR, true, 5_000, 5_000);
    assert_eq!(step, OutboundStep::Discontinuity(None));
}
