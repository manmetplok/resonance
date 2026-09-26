//! Recording-ring overflow reporting (silent-data-loss fix).
//!
//! `SharedState::recording_overflow` counts the whole input frames the
//! RT capture producers discarded because the recording ring was full.
//! It used to be a write-only flag: no event, no UI, no reset — a take
//! missing audio looked exactly like a healthy one. The engine control
//! thread now polls the counter alongside the recording drain
//! ([`RecordingState::poll_overflow`]) and raises
//! `AudioEvent::RecordingOverflow` once per take; a new take re-arms
//! the report and zeroes the count
//! ([`RecordingState::begin_overflow_episode`]).
//!
//! Like `loop_record_takes.rs`, these drive `RecordingState` directly —
//! the same engine-control-thread routines the loop wires up — so the
//! latch/reset contract is exercised without an audio engine or a real
//! input device.

use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_channel::TryRecvError;
use resonance_audio::types::AudioEvent;
use resonance_audio::RecordingState;

#[test]
fn overflow_is_reported_once_with_the_dropped_count() {
    let mut rec = RecordingState::new(48_000);
    let dropped = AtomicU64::new(0);
    let (tx, rx) = crossbeam_channel::unbounded::<AudioEvent>();

    // Clean take: nothing to report, no matter how often it is polled.
    rec.poll_overflow(&dropped, &tx);
    rec.poll_overflow(&dropped, &tx);
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));

    // The producers dropped 1234 frames since the last drain poll.
    dropped.store(1234, Ordering::Relaxed);
    rec.poll_overflow(&dropped, &tx);
    match rx.try_recv() {
        Ok(AudioEvent::RecordingOverflow { dropped_frames }) => {
            assert_eq!(dropped_frames, 1234, "the report is quantitative");
        }
        other => panic!("expected RecordingOverflow, got {other:?}"),
    }

    // A sustained overflow keeps growing the counter, but the report is
    // latched: the engine loop polls every iteration and must not flood
    // the event queue with one event per drain.
    dropped.fetch_add(5_000, Ordering::Relaxed);
    rec.poll_overflow(&dropped, &tx);
    rec.poll_overflow(&dropped, &tx);
    assert!(
        matches!(rx.try_recv(), Err(TryRecvError::Empty)),
        "one overflow episode reports exactly once"
    );
}

#[test]
fn a_new_take_starts_clean_and_rearms_the_report() {
    let mut rec = RecordingState::new(48_000);
    let dropped = AtomicU64::new(0);
    let (tx, rx) = crossbeam_channel::unbounded::<AudioEvent>();

    // First take overflows and is reported.
    dropped.store(64, Ordering::Relaxed);
    rec.poll_overflow(&dropped, &tx);
    assert!(matches!(
        rx.try_recv(),
        Ok(AudioEvent::RecordingOverflow { dropped_frames: 64 })
    ));

    // The next take (record start, or a cycle-record seam) begins a
    // fresh episode: the shared counter is zeroed so the new take never
    // inherits the previous take's damage...
    rec.begin_overflow_episode(&dropped);
    assert_eq!(dropped.load(Ordering::Relaxed), 0);
    rec.poll_overflow(&dropped, &tx);
    assert!(
        matches!(rx.try_recv(), Err(TryRecvError::Empty)),
        "a clean new take reports nothing"
    );

    // ...and the latch is re-armed, so damage to the NEW take is
    // reported in its own right, with its own count.
    dropped.store(7, Ordering::Relaxed);
    rec.poll_overflow(&dropped, &tx);
    assert!(matches!(
        rx.try_recv(),
        Ok(AudioEvent::RecordingOverflow { dropped_frames: 7 })
    ));
}
