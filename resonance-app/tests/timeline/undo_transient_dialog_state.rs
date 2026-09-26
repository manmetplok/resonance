//! Transient dialog / editor state is no edit (ARCH-06 A6-4, A-10 pass 2).
//!
//! `TrackMessage::Bounce(_)` and `ComposeMessage::DrumGroups(_)` fell
//! through `_ => UndoAction::Record` in the old classifier, so every
//! click in the bounce-in-place input picker and every keystroke of a
//! drum-pattern rename pushed its own history entry, wiped the redo
//! stack, marked the project dirty and bumped the control revision. None
//! of that state is in the project: `bounce_dialog` and
//! `DrumrollViewState` are session UI. A rename is one gesture, so it is
//! one entry — the commit.

use resonance_app::compose::messages::DrumGroupsMessage;
use resonance_app::compose::ComposeMessage;
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
    assert!(
        app.test_bounce_dialog().is_none(),
        "Cancel closes the dialog"
    );
}

#[test]
fn cancelling_a_running_bounce_is_no_edit() {
    let mut app = app_with_redo(ViewMode::Arrange);
    assert_no_edit(&mut app, vec![bounce(BounceMessage::CancelInProgress)]);
}

fn drums(msg: DrumGroupsMessage) -> Message {
    Message::Compose(ComposeMessage::DrumGroups(msg))
}

#[test]
fn browsing_the_drum_manager_is_no_edit() {
    let mut app = app_with_redo(ViewMode::Compose);
    let pattern = &app.compose_state().drum_patterns[0];
    let pattern_id = pattern.id;
    let group_id = pattern.groups.first().map(|g| g.id).unwrap_or(0);
    let name = pattern.name.clone();

    assert_no_edit(
        &mut app,
        vec![
            drums(DrumGroupsMessage::SelectGroup { group_id }),
            drums(DrumGroupsMessage::SelectPattern { pattern_id }),
            drums(DrumGroupsMessage::OpenManager),
            drums(DrumGroupsMessage::ManagerSelectGroup { group_id }),
            drums(DrumGroupsMessage::ManagerSetFilter("kick".into())),
            drums(DrumGroupsMessage::CloseManager),
            drums(DrumGroupsMessage::BeginRenamePattern { pattern_id }),
            drums(DrumGroupsMessage::UpdateRenamePatternText("x".into())),
            drums(DrumGroupsMessage::CancelRenamePattern),
        ],
    );
    assert_eq!(
        pattern_name(&app, pattern_id),
        name,
        "a cancelled rename changes nothing"
    );
}

fn pattern_name(app: &Resonance, id: u64) -> String {
    app.compose_state()
        .drum_patterns
        .iter()
        .find(|p| p.id == id)
        .expect("pattern exists")
        .name
        .clone()
}

/// Begin, type five characters, commit: one entry, and one undo puts the
/// old name back. It used to be seven entries — the first six restoring
/// nothing the user could see.
#[test]
fn renaming_a_drum_pattern_is_one_entry() {
    let mut app = app_with_redo(ViewMode::Compose);
    let pattern_id = app.compose_state().drum_patterns[0].id;
    let old_name = pattern_name(&app, pattern_id);
    let before = entries(&app);

    let _ = app.update(drums(DrumGroupsMessage::BeginRenamePattern { pattern_id }));
    for text in ["G", "Gr", "Gro", "Groo", "Groov"] {
        let _ = app.update(drums(DrumGroupsMessage::UpdateRenamePatternText(
            text.into(),
        )));
    }
    assert_eq!(entries(&app), before, "typing is not an edit yet");
    let _ = app.update(drums(DrumGroupsMessage::CommitRenamePattern));

    assert_eq!(pattern_name(&app, pattern_id), "Groov");
    assert_eq!(entries(&app), before + 1, "one rename, one entry");
    assert!(app.is_dirty());

    let _ = app.update(Message::Undo);
    assert_eq!(
        pattern_name(&app, pattern_id),
        old_name,
        "one undo restores the old name"
    );
}
