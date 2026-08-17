//! Engine freeze-event mirror (ba todo #575).
//!
//! These tests drive the `Freeze*` [`AudioEvent`]s through the real
//! `handle_engine_event` dispatch and assert the per-track
//! [`FreezeStatus`] transitions plus the batch-queue advancement — i.e.
//! that a freeze run reflected *purely from events* leaves the track in
//! the right state, with no help from the command-side handlers.

use resonance_app::message::{FreezeMessage, Message};
use resonance_app::state::FreezeStatus;
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, TrackId, TrackType};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

/// Build an app with a capturing engine plus a temp project dir so the
/// command-side freeze handlers (used to set up `Freezing` state) can
/// derive a cache path. Returns the app and the temp dir (kept alive).
fn app() -> (Resonance, tempfile::TempDir) {
    let (mut app, _task) = Resonance::new_for_test();
    let _rx = app.test_capture_engine();
    let dir = tempfile::tempdir().expect("temp project dir");
    app.test_set_project_path(dir.path().to_path_buf());
    (app, dir)
}

fn cache_ref(filename: &str, fingerprint: u64) -> FreezeCacheRef {
    FreezeCacheRef::new(
        filename.to_string(),
        48_000,
        32,
        fingerprint,
        // Deliberately not `Frozen` so we can assert the mirror canonicalises
        // a completed ref to `Frozen` regardless of what the engine sent.
        FreezeCacheStatus::Stale,
    )
}

/// Put a track into the `Freezing` state via the real command handler, the
/// same way a user-initiated freeze would before any engine event lands.
fn start_freezing(app: &mut Resonance, track_id: TrackId) {
    app.test_add_track(track_id, TrackType::Instrument);
    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeTrack(track_id)));
    assert_eq!(
        app.test_freeze_status(track_id),
        FreezeStatus::Freezing { fraction: 0.0 },
        "track should be Freezing after the command handler runs"
    );
}

// ---------------------------------------------------------------------
// Single-track lifecycle
// ---------------------------------------------------------------------

#[test]
fn full_freeze_run_from_events_lands_frozen_with_cache_ref() {
    let (mut app, _dir) = app();
    start_freezing(&mut app, 1);

    // Progress updates the in-flight fraction.
    app.test_apply_engine_event(AudioEvent::FreezeProgress {
        track_id: 1,
        fraction: 0.5,
    });
    assert_eq!(
        app.test_freeze_status(1),
        FreezeStatus::Freezing { fraction: 0.5 }
    );

    // Completion stores the cache ref and marks the track Frozen.
    let cr = cache_ref("freeze_1.wav", 0xABCD);
    app.test_apply_engine_event(AudioEvent::FreezeCompleted {
        track_id: 1,
        cache_ref: cr.clone(),
    });

    match app.test_freeze_status(1) {
        FreezeStatus::Frozen { cache_ref } => {
            assert_eq!(cache_ref.cache_filename, "freeze_1.wav");
            assert_eq!(cache_ref.render_fingerprint, 0xABCD);
            // A completed freeze is canonically Frozen even though the
            // engine sent the ref as Stale.
            assert_eq!(cache_ref.status, FreezeCacheStatus::Frozen);
        }
        other => panic!("expected Frozen, got {other:?}"),
    }
    assert!(app.test_freeze_status(1).is_frozen());
}

#[test]
fn progress_is_clamped_to_unit_range() {
    let (mut app, _dir) = app();
    start_freezing(&mut app, 1);

    app.test_apply_engine_event(AudioEvent::FreezeProgress {
        track_id: 1,
        fraction: 1.7,
    });
    assert_eq!(
        app.test_freeze_status(1),
        FreezeStatus::Freezing { fraction: 1.0 }
    );

    app.test_apply_engine_event(AudioEvent::FreezeProgress {
        track_id: 1,
        fraction: -0.4,
    });
    assert_eq!(
        app.test_freeze_status(1),
        FreezeStatus::Freezing { fraction: 0.0 }
    );
}

#[test]
fn error_marks_failed_and_falls_back_to_live() {
    let (mut app, _dir) = app();
    start_freezing(&mut app, 1);

    app.test_apply_engine_event(AudioEvent::FreezeError {
        track_id: 1,
        message: "render blew up".into(),
    });

    assert_eq!(
        app.test_freeze_status(1),
        FreezeStatus::Failed {
            message: "render blew up".into()
        }
    );
    // Failed carries no cache -> the mixer plays the live chain.
    assert!(!app.test_freeze_status(1).is_frozen());
}

#[test]
fn cancel_returns_track_to_live_with_no_cache() {
    let (mut app, _dir) = app();
    start_freezing(&mut app, 1);

    app.test_apply_engine_event(AudioEvent::FreezeCancelled { track_id: 1 });

    assert_eq!(app.test_freeze_status(1), FreezeStatus::Idle);
    assert!(!app.test_freeze_status(1).is_frozen());
}

#[test]
fn late_progress_after_completion_does_not_resurrect_freezing() {
    let (mut app, _dir) = app();
    start_freezing(&mut app, 1);

    app.test_apply_engine_event(AudioEvent::FreezeCompleted {
        track_id: 1,
        cache_ref: cache_ref("freeze_1.wav", 1),
    });
    // A progress event that races past the terminal must be ignored.
    app.test_apply_engine_event(AudioEvent::FreezeProgress {
        track_id: 1,
        fraction: 0.9,
    });
    assert!(matches!(
        app.test_freeze_status(1),
        FreezeStatus::Frozen { .. }
    ));
}

// ---------------------------------------------------------------------
// Batch queue advancement
// ---------------------------------------------------------------------

#[test]
fn completed_event_advances_the_batch_to_the_next_track() {
    let (mut app, _dir) = app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_add_track(2, TrackType::Instrument);

    // Freeze-all builds a queue and starts track 1 rendering.
    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeAllTracks));
    let q = app.test_freeze_queue().expect("batch queue active");
    assert_eq!(q.current, Some(1));
    assert_eq!(q.total, 2);
    assert_eq!(q.completed, 0);

    // First track completes via an engine event: it lands Frozen and the
    // batch rolls onto track 2.
    app.test_apply_engine_event(AudioEvent::FreezeCompleted {
        track_id: 1,
        cache_ref: cache_ref("freeze_1.wav", 1),
    });
    assert!(matches!(
        app.test_freeze_status(1),
        FreezeStatus::Frozen { .. }
    ));
    let q = app.test_freeze_queue().expect("batch still active");
    assert_eq!(q.current, Some(2));
    assert_eq!(q.completed, 1);
    assert_eq!(
        app.test_freeze_status(2),
        FreezeStatus::Freezing { fraction: 0.0 }
    );

    // Second (last) track completes -> both Frozen, queue drained.
    app.test_apply_engine_event(AudioEvent::FreezeCompleted {
        track_id: 2,
        cache_ref: cache_ref("freeze_2.wav", 2),
    });
    assert!(matches!(
        app.test_freeze_status(2),
        FreezeStatus::Frozen { .. }
    ));
    assert!(
        app.test_freeze_queue().is_none(),
        "queue should be dropped once exhausted"
    );
}

#[test]
fn error_advances_the_batch_so_one_failure_does_not_wedge_it() {
    let (mut app, _dir) = app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_add_track(2, TrackType::Instrument);

    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeAllTracks));
    assert_eq!(app.test_freeze_queue().unwrap().current, Some(1));

    // Track 1 errors: it goes Failed, but the batch still rolls to track 2.
    app.test_apply_engine_event(AudioEvent::FreezeError {
        track_id: 1,
        message: "boom".into(),
    });
    assert!(matches!(
        app.test_freeze_status(1),
        FreezeStatus::Failed { .. }
    ));
    let q = app.test_freeze_queue().expect("batch continues after error");
    assert_eq!(q.current, Some(2));
}

#[test]
fn cancel_event_abandons_the_whole_batch() {
    let (mut app, _dir) = app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_add_track(2, TrackType::Instrument);

    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeAllTracks));
    assert!(app.test_freeze_queue().is_some());

    app.test_apply_engine_event(AudioEvent::FreezeCancelled { track_id: 1 });

    assert_eq!(app.test_freeze_status(1), FreezeStatus::Idle);
    assert!(
        app.test_freeze_queue().is_none(),
        "cancel stops the run, it does not skip to the next track"
    );
    // Track 2 never started.
    assert_eq!(app.test_freeze_status(2), FreezeStatus::Idle);
}
