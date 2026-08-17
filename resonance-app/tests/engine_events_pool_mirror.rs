//! Engine-event mirroring tests for the media-pool import + audition
//! lifecycle (doc #175, ba todo #597).
//!
//! All assertions are driven through `test_apply_engine_event` (the real
//! dispatch path) and the read-only test accessors — no private fields.
//!
//! Covered:
//!   * `ImportProgress` (Queued / Working / Done) → per-file tracker
//!   * `ImportFailed` → tracker + error_message
//!   * `AssetImported` → pool upsert (existing coverage; verifying it
//!     still works alongside the new tracker)
//!   * `AuditionPosition` → `browser.audition.position_frame`
//!   * `AuditionStopped` → clears `playing` + resets `position_frame`

use std::path::PathBuf;

use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, ImportStage};
use resonance_common::AudioFormat;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn asset_imported(asset_id: u64) -> AudioEvent {
    AudioEvent::AssetImported {
        asset_id,
        project_relative_path: format!("audio/asset_{asset_id}.wav"),
        original_path: format!("/src/drum_{asset_id}.wav"),
        format: AudioFormat::Wav,
        channels: 2,
        source_sample_rate: 44_100,
        duration_frames: 88_200,
        peaks: vec![(0.1, 0.9)],
    }
}

// ---------------------------------------------------------------------------
// ImportProgress mirroring
// ---------------------------------------------------------------------------

#[test]
fn import_progress_queued_stage_is_tracked() {
    let (mut app, _task) = Resonance::new_for_test();

    app.test_apply_engine_event(AudioEvent::ImportProgress {
        asset_id: 1,
        path: "/src/kick.wav".into(),
        stage: ImportStage::Queued,
    });

    let statuses = app.test_import_progress().statuses();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].asset_id, 1);
    assert_eq!(statuses[0].path, "/src/kick.wav");
    assert!(matches!(
        statuses[0].progress,
        resonance_app::state::FileImportProgress::Queued
    ));
}

#[test]
fn import_progress_working_stage_updates_entry_in_place() {
    let (mut app, _task) = Resonance::new_for_test();

    app.test_apply_engine_event(AudioEvent::ImportProgress {
        asset_id: 2,
        path: "/src/snare.wav".into(),
        stage: ImportStage::Queued,
    });
    app.test_apply_engine_event(AudioEvent::ImportProgress {
        asset_id: 2,
        path: "/src/snare.wav".into(),
        stage: ImportStage::Working,
    });

    // Still one entry — the Queued→Working transition is an in-place update.
    let statuses = app.test_import_progress().statuses();
    assert_eq!(statuses.len(), 1);
    assert!(matches!(
        statuses[0].progress,
        resonance_app::state::FileImportProgress::Working
    ));
}

#[test]
fn import_progress_done_stage_marks_entry_done() {
    let (mut app, _task) = Resonance::new_for_test();

    app.test_apply_engine_event(AudioEvent::ImportProgress {
        asset_id: 3,
        path: "/src/hat.wav".into(),
        stage: ImportStage::Queued,
    });
    app.test_apply_engine_event(AudioEvent::ImportProgress {
        asset_id: 3,
        path: "/src/hat.wav".into(),
        stage: ImportStage::Working,
    });
    app.test_apply_engine_event(AudioEvent::ImportProgress {
        asset_id: 3,
        path: "/src/hat.wav".into(),
        stage: ImportStage::Done,
    });

    let statuses = app.test_import_progress().statuses();
    assert_eq!(statuses.len(), 1);
    assert!(matches!(
        statuses[0].progress,
        resonance_app::state::FileImportProgress::Done
    ));
    assert!(app.test_import_progress().is_complete());
}

#[test]
fn multiple_files_get_independent_entries() {
    let (mut app, _task) = Resonance::new_for_test();

    app.test_apply_engine_event(AudioEvent::ImportProgress {
        asset_id: 10,
        path: "/src/a.wav".into(),
        stage: ImportStage::Queued,
    });
    app.test_apply_engine_event(AudioEvent::ImportProgress {
        asset_id: 11,
        path: "/src/b.wav".into(),
        stage: ImportStage::Working,
    });

    let statuses = app.test_import_progress().statuses();
    assert_eq!(statuses.len(), 2);
    // Order of first appearance is preserved.
    assert_eq!(statuses[0].asset_id, 10);
    assert_eq!(statuses[1].asset_id, 11);
    // Not complete while any entry is Queued or Working.
    assert!(!app.test_import_progress().is_complete());
}

#[test]
fn batch_is_complete_when_all_entries_are_terminal() {
    let (mut app, _task) = Resonance::new_for_test();

    app.test_apply_engine_event(AudioEvent::ImportProgress {
        asset_id: 20,
        path: "/src/loop.wav".into(),
        stage: ImportStage::Done,
    });
    app.test_apply_engine_event(AudioEvent::ImportFailed {
        asset_id: 21,
        path: "/src/missing.wav".into(),
        reason: "file not found".into(),
    });

    assert!(app.test_import_progress().is_complete());
}

// ---------------------------------------------------------------------------
// ImportFailed mirroring
// ---------------------------------------------------------------------------

#[test]
fn import_failed_records_failed_entry_in_tracker() {
    let (mut app, _task) = Resonance::new_for_test();

    app.test_apply_engine_event(AudioEvent::ImportFailed {
        asset_id: 5,
        path: "/src/corrupt.wav".into(),
        reason: "unsupported codec".into(),
    });

    let statuses = app.test_import_progress().statuses();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].asset_id, 5);
    assert_eq!(statuses[0].path, "/src/corrupt.wav");
    match &statuses[0].progress {
        resonance_app::state::FileImportProgress::Failed { reason } => {
            assert_eq!(reason, "unsupported codec");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn import_failed_updates_existing_tracker_entry_in_place() {
    // If ImportProgress Queued arrived first, ImportFailed should
    // transition the same row to Failed (no duplicate entry).
    let (mut app, _task) = Resonance::new_for_test();

    app.test_apply_engine_event(AudioEvent::ImportProgress {
        asset_id: 6,
        path: "/src/flac.flac".into(),
        stage: ImportStage::Queued,
    });
    app.test_apply_engine_event(AudioEvent::ImportFailed {
        asset_id: 6,
        path: "/src/flac.flac".into(),
        reason: "decode error".into(),
    });

    let statuses = app.test_import_progress().statuses();
    assert_eq!(statuses.len(), 1, "no duplicate entry after failure");
    assert!(matches!(
        &statuses[0].progress,
        resonance_app::state::FileImportProgress::Failed { .. }
    ));
}

#[test]
fn import_failed_sets_error_message() {
    let (mut app, _task) = Resonance::new_for_test();

    app.test_apply_engine_event(AudioEvent::ImportFailed {
        asset_id: 7,
        path: "/src/bad.mp3".into(),
        reason: "unexpected EOF".into(),
    });

    // The error banner is populated.
    assert!(app
        .test_error_message()
        .map(|m| m.contains("unexpected EOF"))
        .unwrap_or(false));
}

// ---------------------------------------------------------------------------
// AssetImported still updates the pool (regression guard)
// ---------------------------------------------------------------------------

#[test]
fn asset_imported_adds_asset_to_pool() {
    let (mut app, _task) = Resonance::new_for_test();

    app.test_apply_engine_event(asset_imported(42));

    assert!(app.test_pool().asset(42).is_some());
    assert!(!app.test_pool().asset(42).unwrap().missing);
}

#[test]
fn asset_imported_idempotent_on_duplicate_event() {
    let (mut app, _task) = Resonance::new_for_test();

    app.test_apply_engine_event(asset_imported(43));
    app.test_apply_engine_event(asset_imported(43));

    // Only one asset entry, not two.
    let count = app.test_pool().assets.iter().filter(|a| a.id == 43).count();
    assert_eq!(count, 1);
}

// ---------------------------------------------------------------------------
// AuditionPosition mirroring
// ---------------------------------------------------------------------------

#[test]
fn audition_position_updates_playhead_frame() {
    let (mut app, _task) = Resonance::new_for_test();

    app.test_apply_engine_event(AudioEvent::AuditionPosition { frame: 12_345 });

    assert_eq!(app.test_audition().position_frame, 12_345);
}

#[test]
fn audition_position_is_updated_on_each_event() {
    let (mut app, _task) = Resonance::new_for_test();

    app.test_apply_engine_event(AudioEvent::AuditionPosition { frame: 100 });
    app.test_apply_engine_event(AudioEvent::AuditionPosition { frame: 200 });
    app.test_apply_engine_event(AudioEvent::AuditionPosition { frame: 300 });

    assert_eq!(app.test_audition().position_frame, 300);
}

// ---------------------------------------------------------------------------
// AuditionStopped mirroring
// ---------------------------------------------------------------------------

#[test]
fn audition_stopped_clears_playing_row_and_resets_position() {
    let (mut app, _task) = Resonance::new_for_test();

    // Simulate a playing state by going through the browser update path.
    // The simplest way without the browser handler is to note that the
    // engine can send AuditionPosition while playing; we inject that state
    // using the existing test accessor indirectly via engine events.
    //
    // We directly poke the browser state via test_set_audition_playing so
    // that AuditionStopped has something to clear.
    app.test_set_audition_playing(Some(PathBuf::from("/src/preview.wav")));
    app.test_apply_engine_event(AudioEvent::AuditionPosition { frame: 44_100 });

    assert!(app.test_audition().playing.is_some());
    assert_eq!(app.test_audition().position_frame, 44_100);

    app.test_apply_engine_event(AudioEvent::AuditionStopped);

    assert!(app.test_audition().playing.is_none());
    assert_eq!(app.test_audition().position_frame, 0);
}

#[test]
fn audition_stopped_when_nothing_playing_is_a_noop() {
    let (mut app, _task) = Resonance::new_for_test();

    // Nothing playing — must not panic or corrupt state.
    app.test_apply_engine_event(AudioEvent::AuditionStopped);

    assert!(app.test_audition().playing.is_none());
    assert_eq!(app.test_audition().position_frame, 0);
}
