//! Opening the delete-track confirm is no edit (code review STATE-13).
//!
//! `RequestRemoveTrack` on a track with clips only opens the confirmation
//! dialog, but it was classified `Record`: opening the dialog pushed an
//! undo entry, cleared redo, marked the project dirty and bumped the
//! revision — and a Cancel left all of that behind. The request is now
//! `Skip`; the delete itself (`ConfirmRemoveTrack`, which an empty track
//! re-dispatches straight away) is the one recorded edit.

use resonance_app::message::{Message, TrackMessage};
use resonance_app::state::{ClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, FadeCurve, TrackType};

const TRACK: u64 = 1;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    // The history only records once the project has a path on disk.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/mixer-delete-confirm.rprj"));
    app.test_add_track(TRACK, TrackType::Audio);
    app
}

fn clip() -> ClipState {
    ClipState {
        id: 7,
        track_id: TRACK,
        start_sample: 0,
        duration_samples: 48_000,
        name: "clip".into(),
        total_frames: 48_000,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
        warp: Default::default(),
    }
}

fn entries(app: &Resonance) -> usize {
    app.test_undo_history().test_undo_entries().len()
}

#[test]
fn opening_and_cancelling_the_confirm_records_nothing() {
    let mut app = app();
    app.test_push_clip(clip());
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -6.0)));
    let _ = app.update(Message::Undo);
    assert!(app.test_undo_history().can_redo());
    app.test_set_dirty(false);
    let revision = app.revision();
    let before = entries(&app);

    let _ = app.update(Message::Track(TrackMessage::RequestRemoveTrack(TRACK)));
    let _ = app.update(Message::Track(TrackMessage::CancelRemoveTrack));

    assert_eq!(entries(&app), before, "the dialog is no undo entry");
    assert!(!app.is_dirty(), "the dialog does not dirty the project");
    assert_eq!(app.revision(), revision, "the dialog is no edit");
    assert!(app.test_undo_history().can_redo(), "redo survives the dialog");
}

#[test]
fn confirming_the_delete_is_exactly_one_entry() {
    let mut app = app();
    app.test_push_clip(clip());
    let revision = app.revision();

    let _ = app.update(Message::Track(TrackMessage::RequestRemoveTrack(TRACK)));
    let _ = app.update(Message::Track(TrackMessage::ConfirmRemoveTrack));

    assert_eq!(entries(&app), 1);
    assert_eq!(app.revision(), revision + 1);
}

#[test]
fn an_empty_track_is_deleted_at_once_as_one_entry() {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/mixer-delete-confirm.rprj"));
    app.test_add_track(TRACK, TrackType::Audio);
    let revision = app.revision();

    let _ = app.update(Message::Track(TrackMessage::RequestRemoveTrack(TRACK)));

    let sent: Vec<AudioCommand> = rx.try_iter().collect();
    assert!(
        sent.iter()
            .any(|c| matches!(c, AudioCommand::RemoveTrack { track_id } if *track_id == TRACK)),
        "an empty track needs no confirm"
    );
    assert_eq!(entries(&app), 1, "the delete is one undo entry");
    assert_eq!(app.revision(), revision + 1);
    assert!(app.is_dirty());
}
