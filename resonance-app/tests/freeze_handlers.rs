//! Track-freeze update handlers + undo wiring (ba todo #574).
//!
//! Each [`FreezeMessage`] is dispatched against a `Resonance` whose engine
//! has been swapped for a command-capturing stub, so the tests assert the
//! exact `AudioCommand`(s) the handler emits plus the app-side
//! [`FreezeStatus`] transition. The engine owns the actual render and its
//! progress / completion events (mirrored by ba todo #575) drive the later
//! transitions, so those are simulated here with `test_set_freeze_status`.

use std::collections::HashMap;
use std::path::PathBuf;

use resonance_app::message::{FreezeMessage, Message};
use resonance_app::state::FreezeStatus;
use resonance_app::undo::{classify, UndoAction};
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::types::{AudioCommand, TrackId, TrackType};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

/// Build an app with a capturing engine plus a temp project directory so
/// the freeze handlers can derive a cache path. Returns the app, the
/// command receiver, and the temp dir (kept alive for the test's lifetime).
fn capturing_app() -> (Resonance, Receiver<AudioCommand>, tempfile::TempDir) {
    let (mut app, _task) = Resonance::new();
    let rx = app.test_capture_engine();
    let dir = tempfile::tempdir().expect("temp project dir");
    app.test_set_project_path(dir.path().to_path_buf());
    (app, rx, dir)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn cache_ref(filename: &str) -> FreezeCacheRef {
    FreezeCacheRef::new(filename.to_string(), 48_000, 32, 0, FreezeCacheStatus::Frozen)
}

/// Path the handler writes a track's freeze cache to, given the project dir.
fn cache_path(project_dir: &std::path::Path, track_id: TrackId) -> PathBuf {
    project_dir
        .join("freeze")
        .join(format!("freeze_{track_id}.wav"))
}

// ---------------------------------------------------------------------
// Single-track freeze
// ---------------------------------------------------------------------

#[test]
fn freeze_track_emits_command_and_marks_freezing() {
    let (mut app, rx, dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);

    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeTrack(1)));

    let cmds = drain(&rx);
    let expected = cache_path(dir.path(), 1);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            AudioCommand::FreezeTrack { track_id, cache_path }
                if *track_id == 1 && std::path::Path::new(cache_path) == expected
        )),
        "expected FreezeTrack into {expected:?}, got {cmds:?}",
    );
    assert_eq!(
        app.test_freeze_status(1),
        FreezeStatus::Freezing { fraction: 0.0 }
    );
    // The handler creates the freeze cache directory up front so the
    // engine's WAV writer (which doesn't mkdir) can write into it.
    assert!(dir.path().join("freeze").is_dir());
}

#[test]
fn freeze_vocal_track_allowed() {
    let (mut app, rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Vocal);

    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeTrack(1)));

    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::FreezeTrack { track_id: 1, .. })));
    assert!(app.test_freeze_status(1).is_freezing());
}

#[test]
fn freeze_audio_track_rejected() {
    let (mut app, rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Audio);

    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeTrack(1)));

    assert!(
        drain(&rx).is_empty(),
        "an audio track has no live synth to freeze",
    );
    assert_eq!(app.test_freeze_status(1), FreezeStatus::Idle);
}

#[test]
fn freeze_without_saved_project_fails() {
    let (mut app, _task) = Resonance::new();
    let rx = app.test_capture_engine();
    // No project path set — there's nowhere to write the cache.
    app.test_add_track(1, TrackType::Instrument);

    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeTrack(1)));

    assert!(drain(&rx).is_empty());
    assert!(matches!(
        app.test_freeze_status(1),
        FreezeStatus::Failed { .. }
    ));
}

#[test]
fn freeze_while_playing_rejected() {
    let (mut app, rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_set_transport_playing(true);

    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeTrack(1)));

    assert!(drain(&rx).is_empty());
    assert_eq!(app.test_freeze_status(1), FreezeStatus::Idle);
}

#[test]
fn freeze_already_freezing_is_noop() {
    let (mut app, rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_set_freeze_status(1, FreezeStatus::Freezing { fraction: 0.3 });

    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeTrack(1)));

    assert!(drain(&rx).is_empty(), "a repeat freeze request is ignored");
}

// ---------------------------------------------------------------------
// Unfreeze
// ---------------------------------------------------------------------

#[test]
fn unfreeze_detaches_and_deletes_cache() {
    let (mut app, rx, dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    // Simulate a completed freeze: status frozen + a real cache file.
    let cache_dir = dir.path().join("freeze");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let file = cache_dir.join("freeze_1.wav");
    std::fs::write(&file, b"fake wav").unwrap();
    app.test_set_freeze_status(
        1,
        FreezeStatus::Frozen {
            cache_ref: cache_ref("freeze_1.wav"),
        },
    );

    app.test_dispatch(Message::Freeze(FreezeMessage::UnfreezeTrack(1)));

    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::UnfreezeTrack { track_id: 1 })));
    assert!(!file.exists(), "the cache file is deleted on unfreeze");
    assert_eq!(app.test_freeze_status(1), FreezeStatus::Idle);
}

#[test]
fn unfreeze_idle_track_is_noop() {
    let (mut app, rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);

    app.test_dispatch(Message::Freeze(FreezeMessage::UnfreezeTrack(1)));

    assert!(drain(&rx).is_empty());
    assert_eq!(app.test_freeze_status(1), FreezeStatus::Idle);
}

// ---------------------------------------------------------------------
// Cancel
// ---------------------------------------------------------------------

#[test]
fn cancel_freeze_emits_cancel_and_rolls_back() {
    let (mut app, rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_add_track(2, TrackType::Instrument);
    // Start a batch so there's a queue + an in-flight track.
    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeAllTracks));
    let _ = drain(&rx);
    assert!(app.test_freeze_queue().is_some());

    app.test_dispatch(Message::Freeze(FreezeMessage::CancelFreeze));

    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::CancelFreeze)));
    assert_eq!(app.test_freeze_status(1), FreezeStatus::Idle);
    assert!(app.test_freeze_queue().is_none());
}

// ---------------------------------------------------------------------
// Batch freeze + queue
// ---------------------------------------------------------------------

#[test]
fn freeze_all_queues_every_freezable_track_and_starts_first() {
    let (mut app, rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_add_track(2, TrackType::Vocal);
    app.test_add_track(3, TrackType::Instrument);
    app.test_add_track(4, TrackType::Audio); // not freezable

    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeAllTracks));

    let queue = app.test_freeze_queue().expect("batch queue");
    assert_eq!(queue.total, 3, "audio track is excluded");
    assert_eq!(queue.current, Some(1));
    assert_eq!(queue.pending, [2, 3]);
    assert_eq!(queue.completed, 0);

    assert!(app.test_freeze_status(1).is_freezing());
    assert_eq!(app.test_freeze_status(2), FreezeStatus::Idle);

    let freeze_cmds = drain(&rx)
        .into_iter()
        .filter(|c| matches!(c, AudioCommand::FreezeTrack { .. }))
        .count();
    assert_eq!(freeze_cmds, 1, "only the first track starts rendering");
}

#[test]
fn advance_freeze_queue_walks_through_the_batch() {
    let (mut app, rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_add_track(2, TrackType::Instrument);
    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeAllTracks));
    let _ = drain(&rx);

    // Track 1 completes (mirror sets it frozen), then we advance.
    app.test_set_freeze_status(
        1,
        FreezeStatus::Frozen {
            cache_ref: cache_ref("freeze_1.wav"),
        },
    );
    assert!(app.test_advance_freeze_queue());
    let queue = app.test_freeze_queue().expect("still draining");
    assert_eq!(queue.current, Some(2));
    assert_eq!(queue.completed, 1);
    assert!(app.test_freeze_status(2).is_freezing());

    // Track 2 completes; the next advance exhausts the batch.
    app.test_set_freeze_status(
        2,
        FreezeStatus::Frozen {
            cache_ref: cache_ref("freeze_2.wav"),
        },
    );
    assert!(!app.test_advance_freeze_queue());
    assert!(app.test_freeze_queue().is_none(), "queue dropped when done");
}

#[test]
fn freeze_all_skips_already_frozen_tracks() {
    let (mut app, rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_add_track(2, TrackType::Instrument);
    app.test_set_freeze_status(
        1,
        FreezeStatus::Frozen {
            cache_ref: cache_ref("freeze_1.wav"),
        },
    );

    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeAllTracks));
    let _ = drain(&rx);

    let queue = app.test_freeze_queue().expect("batch queue");
    assert_eq!(queue.total, 1);
    assert_eq!(queue.current, Some(2));
}

#[test]
fn freeze_selected_uses_the_selected_track() {
    let (mut app, rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_add_track(2, TrackType::Instrument);
    app.test_select_track(2);

    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeSelectedTracks));
    let _ = drain(&rx);

    let queue = app.test_freeze_queue().expect("batch queue");
    assert_eq!(queue.total, 1);
    assert_eq!(queue.current, Some(2));
    assert!(app.test_freeze_status(2).is_freezing());
}

// ---------------------------------------------------------------------
// Undo classification
// ---------------------------------------------------------------------

#[test]
fn freeze_actions_are_classified_as_atomic_records() {
    for msg in [
        FreezeMessage::FreezeTrack(1),
        FreezeMessage::UnfreezeTrack(1),
        FreezeMessage::RefreezeTrack(1),
        FreezeMessage::FreezeSelectedTracks,
        FreezeMessage::FreezeAllTracks,
    ] {
        assert!(
            matches!(classify(&Message::Freeze(msg.clone())), UndoAction::Record),
            "{msg:?} should be an atomic undo entry",
        );
    }
}

#[test]
fn cancel_freeze_is_not_undoable() {
    assert!(matches!(
        classify(&Message::Freeze(FreezeMessage::CancelFreeze)),
        UndoAction::Skip
    ));
}

// ---------------------------------------------------------------------
// Undo / redo restore reconciliation
// ---------------------------------------------------------------------

#[test]
fn undo_of_freeze_detaches_and_removes_cache() {
    let (mut app, rx, dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    let cache_dir = dir.path().join("freeze");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let file = cache_dir.join("freeze_1.wav");
    std::fs::write(&file, b"fake wav").unwrap();
    app.test_set_freeze_status(
        1,
        FreezeStatus::Frozen {
            cache_ref: cache_ref("freeze_1.wav"),
        },
    );

    // Undo restores the pre-freeze snapshot: an empty freeze map.
    app.test_apply_freeze_restore(HashMap::new());

    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::UnfreezeTrack { track_id: 1 })));
    assert!(!file.exists(), "cache is not part of undo history");
    assert_eq!(app.test_freeze_status(1), FreezeStatus::Idle);
}

#[test]
fn redo_of_freeze_marks_stale_when_cache_is_gone() {
    let (mut app, _rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    // Redo target wants the track frozen, but the cache file was deleted
    // by the matching undo — the redo can only mark it stale.
    let mut target = HashMap::new();
    target.insert(
        1,
        FreezeStatus::Frozen {
            cache_ref: cache_ref("freeze_1.wav"),
        },
    );

    app.test_apply_freeze_restore(target);

    assert!(
        matches!(app.test_freeze_status(1), FreezeStatus::Stale { .. }),
        "missing cache downgrades a restored freeze to stale",
    );
}

#[test]
fn redo_of_freeze_keeps_frozen_when_cache_exists() {
    let (mut app, _rx, dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    let cache_dir = dir.path().join("freeze");
    std::fs::create_dir_all(&cache_dir).unwrap();
    std::fs::write(cache_dir.join("freeze_1.wav"), b"fake wav").unwrap();
    let mut target = HashMap::new();
    target.insert(
        1,
        FreezeStatus::Frozen {
            cache_ref: cache_ref("freeze_1.wav"),
        },
    );

    app.test_apply_freeze_restore(target);

    assert!(matches!(
        app.test_freeze_status(1),
        FreezeStatus::Frozen { .. }
    ));
}

// ---------------------------------------------------------------------
// Status → persisted projection (consumed by the project-IO slice #577)
// ---------------------------------------------------------------------

#[test]
fn status_projects_onto_persisted_freeze_state() {
    let frozen = FreezeStatus::Frozen {
        cache_ref: cache_ref("freeze_1.wav"),
    };
    let persisted = frozen.to_persisted();
    assert!(persisted.is_frozen);
    assert_eq!(
        persisted.cache_ref.unwrap().status,
        FreezeCacheStatus::Frozen
    );

    let stale = FreezeStatus::Stale {
        cache_ref: cache_ref("freeze_1.wav"),
    };
    let persisted = stale.to_persisted();
    assert!(persisted.is_frozen);
    assert_eq!(persisted.cache_ref.unwrap().status, FreezeCacheStatus::Stale);

    assert!(!FreezeStatus::Idle.to_persisted().is_frozen);
    assert!(!FreezeStatus::Freezing { fraction: 0.5 }
        .to_persisted()
        .is_frozen);
}
