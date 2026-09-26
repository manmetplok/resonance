//! Transient dialog / editor state is no edit (ARCH-06 A6-4, A-10 pass 2).
//!
//! `TrackMessage::Bounce(_)` fell through `_ => UndoAction::Record` in
//! the old classifier, so every click in the bounce-in-place input picker
//! pushed its own history entry, wiped the redo stack, marked the project
//! dirty and bumped the control revision. None of that state is in the
//! project: `bounce_dialog` is session UI.

use resonance_app::message::{BounceMessage, Message, TrackMessage};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::TrackType;

const TRACK: u64 = 1;

/// An app with one edit undone, so the redo stack holds an entry, and a
/// clean dirty flag.
fn app_with_redo(view: ViewMode) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(view);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/a10-transient-undo.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -6.0)));
    let _ = app.update(Message::Undo);
    assert!(app.test_undo_history().can_redo());
    app.test_set_dirty(false);
    app
}

fn entries(app: &Resonance) -> usize {
    app.test_undo_history().test_undo_entries().len()
}

/// Dispatch `msgs` in order and assert none of them was an edit.
fn assert_no_edit(app: &mut Resonance, msgs: Vec<Message>) {
    let revision = app.revision();
    let before = entries(app);
    for msg in msgs {
        let label = format!("{msg:?}");
        let _ = app.update(msg);
        assert_eq!(entries(app), before, "{label} pushed an undo entry");
        assert!(app.test_undo_history().can_redo(), "{label} wiped redo");
        assert!(!app.is_dirty(), "{label} marked the project dirty");
        assert_eq!(app.revision(), revision, "{label} bumped the revision");
    }
}

fn bounce(msg: BounceMessage) -> Message {
    Message::Track(TrackMessage::Bounce(msg))
}

#[test]
fn bounce_dialog_choices_are_no_edit() {
    let mut app = app_with_redo(ViewMode::Arrange);
    app.test_open_bounce_dialog(TRACK);

    assert_no_edit(
        &mut app,
        vec![
            bounce(BounceMessage::PickDevice(Some("Scarlett".into()))),
            bounce(BounceMessage::PickPort(2)),
            bounce(BounceMessage::SetMono(true)),
        ],
    );
    let dialog = app.test_bounce_dialog().expect("the dialog is still open");
    assert_eq!(dialog.selected_device.as_deref(), Some("Scarlett"));
    assert_eq!(dialog.selected_port, 2);
    assert!(dialog.mono);

    assert_no_edit(&mut app, vec![bounce(BounceMessage::Cancel)]);
    assert!(app.test_bounce_dialog().is_none(), "Cancel closes the dialog");
}

#[test]
fn cancelling_a_running_bounce_is_no_edit() {
    let mut app = app_with_redo(ViewMode::Arrange);
    assert_no_edit(&mut app, vec![bounce(BounceMessage::CancelInProgress)]);
}
