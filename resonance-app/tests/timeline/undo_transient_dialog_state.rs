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

// ---------------------------------------------------------------------------
// FU-A10a: text fields and knobs coalesce into one entry per gesture
// ---------------------------------------------------------------------------
//
// Unlike the drum-pattern rename above, these fields have no begin/commit
// pair around them — the view dispatches one message per keystroke or per
// slider step straight into the project, so without a `CoalesceKey` each
// one recorded (and dirtied, and bumped the revision, and wiped redo) on
// its own.

fn track_name(app: &Resonance, id: resonance_audio::types::TrackId) -> String {
    app.test_registry()
        .tracks
        .iter()
        .find(|t| t.id == id)
        .expect("track exists")
        .name
        .clone()
}

fn set_name(id: resonance_audio::types::TrackId, name: &str) -> Message {
    Message::Track(TrackMessage::SetTrackName(id, name.to_string()))
}

/// Five keystrokes into the same track's name field is one undo entry,
/// and one undo restores the pre-typing name.
#[test]
fn typing_a_track_name_coalesces_into_one_entry() {
    let mut app = app_with_redo(ViewMode::Arrange);
    let old_name = track_name(&app, TRACK);
    let before = entries(&app);

    for text in ["D", "Dr", "Dru", "Drum", "Drums"] {
        let _ = app.update(set_name(TRACK, text));
    }

    assert_eq!(track_name(&app, TRACK), "Drums");
    assert_eq!(entries(&app), before + 1, "one field, one entry");

    let _ = app.update(Message::Undo);
    assert_eq!(
        track_name(&app, TRACK),
        old_name,
        "one undo restores the pre-typing name"
    );
}

/// Renaming two different tracks records two entries — the coalesce key
/// is per track, so moving to another field's control breaks the run.
#[test]
fn renaming_two_different_tracks_is_two_entries() {
    let mut app = app_with_redo(ViewMode::Arrange);
    const OTHER: u64 = 2;
    app.test_add_track(OTHER, TrackType::Instrument);
    let before = entries(&app);

    let _ = app.update(set_name(TRACK, "Kick"));
    let _ = app.update(set_name(OTHER, "Snare"));

    assert_eq!(entries(&app), before + 2, "two tracks, two entries");
    assert_eq!(track_name(&app, TRACK), "Kick");
    assert_eq!(track_name(&app, OTHER), "Snare");
}

fn group_id_of_first_pattern(app: &Resonance) -> u64 {
    app.compose_state().drum_patterns[0]
        .groups
        .first()
        .map(|g| g.id)
        .unwrap_or(0)
}

fn group_density(app: &Resonance, group_id: u64) -> f32 {
    app.compose_state()
        .drum_patterns
        .iter()
        .flat_map(|p| p.groups.iter())
        .find(|g| g.id == group_id)
        .expect("group exists")
        .density
}

fn group_name(app: &Resonance, group_id: u64) -> String {
    app.compose_state()
        .drum_patterns
        .iter()
        .flat_map(|p| p.groups.iter())
        .find(|g| g.id == group_id)
        .expect("group exists")
        .name
        .clone()
}

/// Five steps of the same drum-group knob (density) is one undo entry,
/// and one undo restores the pre-drag value.
#[test]
fn drum_group_knob_steps_coalesce_into_one_entry() {
    let mut app = app_with_redo(ViewMode::Compose);
    let group_id = group_id_of_first_pattern(&app);
    let old_density = group_density(&app, group_id);
    let before = entries(&app);

    for density in [0.1, 0.2, 0.3, 0.4, 0.5] {
        let _ = app.update(drums(DrumGroupsMessage::SetGroupDensity { group_id, density }));
    }

    assert_eq!(group_density(&app, group_id), 0.5);
    assert_eq!(entries(&app), before + 1, "one knob, one entry");

    let _ = app.update(Message::Undo);
    assert_eq!(
        group_density(&app, group_id),
        old_density,
        "one undo restores the pre-drag density"
    );
}

/// Switching from one knob to another on the same group breaks the
/// coalesce run — each knob has its own key, so this is two entries.
#[test]
fn switching_drum_group_knob_breaks_the_coalesce_run() {
    let mut app = app_with_redo(ViewMode::Compose);
    let group_id = group_id_of_first_pattern(&app);
    let before = entries(&app);

    let _ = app.update(drums(DrumGroupsMessage::SetGroupDensity {
        group_id,
        density: 0.4,
    }));
    let _ = app.update(drums(DrumGroupsMessage::SetGroupSwing {
        group_id,
        swing: 0.4,
    }));

    assert_eq!(entries(&app), before + 2, "different knobs, two entries");
}

/// The manager modal's group-name field has no begin/commit pair (unlike
/// the pattern chip's inline rename above) — it dispatches straight into
/// the project on every keystroke, so it needs its own coalesce key.
#[test]
fn typing_a_drum_group_name_coalesces_into_one_entry() {
    let mut app = app_with_redo(ViewMode::Compose);
    let group_id = group_id_of_first_pattern(&app);
    let old_name = group_name(&app, group_id);
    let before = entries(&app);

    for text in ["K", "Ki", "Kic", "Kick"] {
        let _ = app.update(drums(DrumGroupsMessage::RenameGroup {
            group_id,
            name: text.to_string(),
        }));
    }

    assert_eq!(group_name(&app, group_id), "Kick");
    assert_eq!(entries(&app), before + 1, "one field, one entry");

    let _ = app.update(Message::Undo);
    assert_eq!(group_name(&app, group_id), old_name);
}

fn add_external_track_with_clip(app: &mut Resonance, id: resonance_audio::types::TrackId) {
    app.test_add_track(id, TrackType::Instrument);
    app.test_registry_mut()
        .tracks
        .iter_mut()
        .find(|t| t.id == id)
        .expect("track exists")
        .midi_output_device = Some("Synth Out".to_string());
    app.test_push_midi_clip(resonance_app::state::MidiClipState {
        id: 900,
        track_id: id,
        start_sample: 0,
        duration_ticks: 4 * 480,
        name: "riff".to_string(),
        notes: vec![resonance_audio::types::MidiNote {
            note: 60,
            velocity: 0.8,
            start_tick: 0,
            duration_ticks: 480,
        }],
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
}

/// `BounceInPlace` on an external-MIDI track only opens the realtime
/// picker dialog — it must record nothing until `Bounce(Confirm)`
/// actually starts the render. It used to record unconditionally, so a
/// GUI-driven external bounce was two undo entries (one empty, for
/// opening the dialog) for one user action.
#[test]
fn bounce_in_place_on_external_track_is_one_entry() {
    let mut app = app_with_redo(ViewMode::Arrange);
    const EXT_TRACK: u64 = 3;
    add_external_track_with_clip(&mut app, EXT_TRACK);
    let before = entries(&app);

    let _ = app.update(Message::Track(TrackMessage::BounceInPlace(EXT_TRACK)));
    assert!(
        app.test_bounce_dialog().is_some(),
        "routes to the realtime picker dialog"
    );
    assert_eq!(entries(&app), before, "opening the dialog is no edit yet");

    let _ = app.update(bounce(BounceMessage::PickDevice(Some("Input 1".to_string()))));
    let _ = app.update(bounce(BounceMessage::Confirm));

    assert_eq!(entries(&app), before + 1, "one bounce, one entry");
}
