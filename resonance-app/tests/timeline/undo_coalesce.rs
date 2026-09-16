//! Coalesced undo through the full update path (fader / pan drags).
//!
//! `record_undo` used to build a full project snapshot for every
//! coalesced message and throw it away whenever the message merely
//! continued an existing run — O(project) work per slider event. The
//! snapshot is now only built when a run starts, so these tests pin the
//! observable contract: one entry per drag, undo restores the pre-drag
//! state (the run-opening snapshot), and an intervening edit breaks the
//! run.

use resonance_app::message::{Message, TrackMessage};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::TrackType;

const TRACK: u64 = 1;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    // The history only records once the project has a path on disk.
    app.test_set_project_path(std::path::PathBuf::from(
        "/tmp/timeline-undo-coalesce.rprj",
    ));
    app.test_add_track(TRACK, TrackType::Instrument);
    app
}

fn volume(app: &Resonance) -> f32 {
    app.test_registry()
        .tracks
        .iter()
        .find(|t| t.id == TRACK)
        .expect("test track")
        .volume
}

fn pan(app: &Resonance) -> f32 {
    app.test_registry()
        .tracks
        .iter()
        .find(|t| t.id == TRACK)
        .expect("test track")
        .pan
}

#[test]
fn a_fader_drag_coalesces_into_one_entry_that_restores_the_predrag_state() {
    let mut app = app();
    for db in [-2.0, -4.0, -6.0] {
        let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, db)));
    }
    assert!((volume(&app) - -6.0).abs() < 1e-4);
    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        1,
        "the whole drag is one undo entry"
    );

    let _ = app.update(Message::Undo);
    assert!(
        volume(&app).abs() < 1e-4,
        "undo lands on the pre-drag state, not a mid-drag value"
    );

    let _ = app.update(Message::Redo);
    assert!(
        (volume(&app) - -6.0).abs() < 1e-4,
        "redo restores the drag's final value"
    );
}

// ---- Compound groups through the full update path --------------------
//
// `with_compound_undo` is the control API's per-call atomicity: a
// multi-dispatch handler wraps its dispatches so they land as ONE
// history entry and ONE revision bump. These tests pin the mechanism's
// contract at the app level; the per-handler behaviour lives in the
// control test group.

#[test]
fn a_compound_group_with_zero_mutations_records_and_bumps_nothing() {
    let mut app = app();
    let revision = app.revision();
    app.with_compound_undo(|_app| {});
    assert_eq!(app.revision(), revision, "no mutation, no revision bump");
    assert_eq!(app.test_undo_history().test_undo_entries().len(), 0);
    assert!(!app.test_undo_history().in_compound(), "the group closed");
}

#[test]
fn a_compound_group_with_one_mutation_behaves_like_a_plain_record() {
    let mut app = app();
    let revision = app.revision();
    app.with_compound_undo(|app| {
        let _ = app.update(Message::Track(TrackMessage::SetTrackPan(TRACK, 0.5)));
    });
    assert_eq!(app.revision(), revision + 1);
    assert_eq!(app.test_undo_history().test_undo_entries().len(), 1);

    let _ = app.update(Message::Undo);
    assert!(pan(&app).abs() < 1e-4, "undo restores the pre-group state");
}

#[test]
fn a_multi_edit_compound_group_is_one_entry_restoring_the_precall_state() {
    let mut app = app();
    let revision = app.revision();
    app.with_compound_undo(|app| {
        let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -6.0)));
        let _ = app.update(Message::Track(TrackMessage::SetTrackPan(TRACK, 0.5)));
    });
    assert_eq!(app.revision(), revision + 1, "one bump for the whole group");
    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        1,
        "both edits land in one entry"
    );

    let _ = app.update(Message::Undo);
    assert!(pan(&app).abs() < 1e-4, "one undo restores BOTH edits");
    assert!(volume(&app).abs() < 1e-4);

    let _ = app.update(Message::Redo);
    assert!((pan(&app) - 0.5).abs() < 1e-4, "one redo replays the group");
    assert!((volume(&app) - -6.0).abs() < 1e-4);
}

/// A control call landing mid-fader-drag must not merge into the user's
/// gesture (or vice versa): the group breaks the coalesce run in both
/// directions, so the call undoes alone.
#[test]
fn a_compound_group_does_not_merge_with_a_surrounding_drag() {
    let mut app = app();
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -2.0)));
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -4.0)));
    app.with_compound_undo(|app| {
        // Same coalesce key as the drag above — still its own entry.
        let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -8.0)));
    });
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -12.0)));
    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        3,
        "drag run, compound call, resumed run"
    );

    let _ = app.update(Message::Undo);
    assert!(
        (volume(&app) - -8.0).abs() < 1e-4,
        "the resumed run undoes alone"
    );
    let _ = app.update(Message::Undo);
    assert!(
        (volume(&app) - -4.0).abs() < 1e-4,
        "the compound call undoes alone"
    );
}

#[test]
fn an_edit_on_another_control_breaks_the_coalesce_run() {
    let mut app = app();
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -2.0)));
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -4.0)));
    let _ = app.update(Message::Track(TrackMessage::SetTrackPan(TRACK, 0.5)));
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -8.0)));
    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        3,
        "volume run, pan edit, second volume run"
    );

    let _ = app.update(Message::Undo);
    assert!(
        (volume(&app) - -4.0).abs() < 1e-4,
        "the second volume run undoes alone"
    );
    assert!((pan(&app) - 0.5).abs() < 1e-4);

    let _ = app.update(Message::Undo);
    assert!(pan(&app).abs() < 1e-4, "the pan edit undoes next");
    assert!((volume(&app) - -4.0).abs() < 1e-4);

    let _ = app.update(Message::Undo);
    assert!(
        volume(&app).abs() < 1e-4,
        "the first drag restores the original state"
    );
}
