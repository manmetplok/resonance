//! Recording-alignment latch (doc #260 finding #2, ba todo #1129).
//!
//! Take placement used to be stamped when the Record command was
//! dispatched — before the input stream was even built (an up-to-500 ms
//! open) — so a variable stream-open gap plus the unmeasured I/O round
//! trip landed inside every take. Now the transport starts only after
//! the stream is up, and the capture callback's *first* push latches
//! the raw playhead through `SharedState::latch_recording_start`; the
//! engine loop derives the take start from it (minus the measured
//! capture+playback latency for performer sessions). These tests pin
//! the latch machinery itself — one-shot arming, first-push capture,
//! and idempotent subsequent pushes.

use std::sync::atomic::Ordering;

use resonance_audio::test_support::SharedState;

#[test]
fn first_push_latches_the_playhead_and_disarms() {
    let shared = SharedState::default();
    shared.playhead.store(48_000, Ordering::Relaxed);
    shared.recording_start_pending.store(true, Ordering::Release);

    // First capture push: the playhead at this instant is latched and
    // the pending flag clears so the engine loop can apply it.
    shared.latch_recording_start();
    assert!(!shared.recording_start_pending.load(Ordering::Acquire));
    assert_eq!(shared.recording_start_latch.load(Ordering::Acquire), 48_000);

    // Later pushes — with the playhead advanced — must not move the
    // latched start; the first captured frame is the session anchor.
    shared.playhead.store(48_128, Ordering::Relaxed);
    shared.latch_recording_start();
    assert_eq!(shared.recording_start_latch.load(Ordering::Acquire), 48_000);
}

#[test]
fn unarmed_pushes_never_latch() {
    // A ping capture (or any push while no session is arming) must not
    // touch the latch.
    let shared = SharedState::default();
    shared.playhead.store(1_000, Ordering::Relaxed);
    shared.recording_start_latch.store(777, Ordering::Relaxed);
    shared.latch_recording_start();
    assert_eq!(shared.recording_start_latch.load(Ordering::Acquire), 777);
}

#[test]
fn rearming_latches_the_new_session_start() {
    // Two back-to-back sessions: each arms `pending` fresh, so each
    // latches its own first-push playhead (no stale carry-over).
    let shared = SharedState::default();

    shared.playhead.store(10_000, Ordering::Relaxed);
    shared.recording_start_pending.store(true, Ordering::Release);
    shared.latch_recording_start();
    assert_eq!(shared.recording_start_latch.load(Ordering::Acquire), 10_000);

    shared.playhead.store(90_000, Ordering::Relaxed);
    shared.recording_start_pending.store(true, Ordering::Release);
    shared.latch_recording_start();
    assert_eq!(shared.recording_start_latch.load(Ordering::Acquire), 90_000);
}
