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

use resonance_app::compose::messages::{ChordInspectorMsg, DrumGroupsMessage, LaneInspectorMsg};
use resonance_app::compose::{ComposeMessage, LaneGeneratorKind};
use resonance_app::message::{BounceMessage, Message, TrackMessage};
use resonance_app::state::ViewMode;
use resonance_app::update::external_instrument::ExternalInstrumentMessage;
use resonance_app::{demo, Resonance};
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

// ---------------------------------------------------------------------------
// FU-A10b: lane/chord inspector sliders and the bulk-lyrics editor coalesce
// ---------------------------------------------------------------------------
//
// Same shape as FU-A10a above: `LaneInspectorMsg`/`ChordInspectorMsg`'s
// numeric sliders and the bulk-lyrics `text_editor` dispatch one message per
// slider step or per editor interaction straight into the project, with no
// begin/commit pair, so without a `CoalesceKey` each one recorded its own
// entry — and every bulk-lyrics interaction, including a bare cursor move,
// recorded even though most of them touch nothing.

/// Demo app pinned to Compose with one edit undone (redo available) and a
/// clean dirty flag, plus the demo's pre-seeded vocal lane identity.
fn app_with_redo_and_vocal_lane() -> (Resonance, u64, resonance_audio::types::TrackId) {
    let mut app = app_with_redo(ViewMode::Compose);
    demo::seed_demo_content(&mut app);
    app.test_set_dirty(false);
    let def_id = app.compose_state().definitions[0].id;
    let track_id = app
        .compose_state()
        .definitions[0]
        .lane_generators
        .keys()
        .copied()
        .next()
        .expect("demo seeds a vocal lane generator");
    (app, def_id, track_id)
}

fn lane_inspector(
    definition_id: u64,
    track_id: resonance_audio::types::TrackId,
    msg: LaneInspectorMsg,
) -> Message {
    Message::Compose(ComposeMessage::LaneInspector {
        definition_id,
        track_id,
        msg,
    })
}

fn vocal_vibrato(app: &Resonance, definition_id: u64, track_id: resonance_audio::types::TrackId) -> f32 {
    let def = app
        .compose_state()
        .definitions
        .iter()
        .find(|d| d.id == definition_id)
        .expect("definition exists");
    match &def
        .lane_generators
        .get(&track_id)
        .expect("lane generator installed")
        .kind
    {
        LaneGeneratorKind::Vocal(p) => p.vibrato,
        other => panic!("expected a Vocal lane generator, got {other:?}"),
    }
}

/// Five steps of the same lane-inspector slider (vocal vibrato) is one undo
/// entry, and one undo restores the pre-drag value.
#[test]
fn lane_inspector_slider_steps_coalesce_into_one_entry() {
    let (mut app, def_id, track_id) = app_with_redo_and_vocal_lane();
    let old_vibrato = vocal_vibrato(&app, def_id, track_id);
    let before = entries(&app);

    for v in [0.1, 0.2, 0.3, 0.4, 0.5] {
        let _ = app.update(lane_inspector(
            def_id,
            track_id,
            LaneInspectorMsg::SetVocalVibrato(v),
        ));
    }

    assert!((vocal_vibrato(&app, def_id, track_id) - 0.5).abs() < 1e-6);
    assert_eq!(entries(&app), before + 1, "one slider, one entry");

    let _ = app.update(Message::Undo);
    assert!(
        (vocal_vibrato(&app, def_id, track_id) - old_vibrato).abs() < 1e-6,
        "one undo restores the pre-drag value"
    );
}

/// Switching from one lane-inspector slider to another breaks the coalesce
/// run — each knob has its own key, so this is two entries.
#[test]
fn switching_lane_inspector_slider_breaks_the_coalesce_run() {
    let (mut app, def_id, track_id) = app_with_redo_and_vocal_lane();
    let before = entries(&app);

    let _ = app.update(lane_inspector(
        def_id,
        track_id,
        LaneInspectorMsg::SetVocalVibrato(0.4),
    ));
    let _ = app.update(lane_inspector(
        def_id,
        track_id,
        LaneInspectorMsg::SetVocalTension(0.4),
    ));

    assert_eq!(entries(&app), before + 2, "different sliders, two entries");
}

fn motif_complexity(app: &Resonance, definition_id: u64) -> f32 {
    app.compose_state()
        .definitions
        .iter()
        .find(|d| d.id == definition_id)
        .expect("definition exists")
        .motif_source
        .params()
        .complexity
}

fn chord_inspector(definition_id: u64, msg: ChordInspectorMsg) -> Message {
    Message::Compose(ComposeMessage::ChordInspector { definition_id, msg })
}

/// Five steps of the same chord-inspector slider (motif complexity) is one
/// undo entry, and one undo restores the pre-drag value.
#[test]
fn chord_inspector_slider_steps_coalesce_into_one_entry() {
    let mut app = app_with_redo(ViewMode::Compose);
    demo::seed_demo_content(&mut app);
    app.test_set_dirty(false);
    let def_id = app.compose_state().definitions[0].id;
    let old_complexity = motif_complexity(&app, def_id);
    let before = entries(&app);

    for c in [0.1, 0.2, 0.3, 0.4, 0.5] {
        let _ = app.update(chord_inspector(
            def_id,
            ChordInspectorMsg::SetMotifComplexity(c),
        ));
    }

    assert!((motif_complexity(&app, def_id) - 0.5).abs() < 1e-6);
    assert_eq!(entries(&app), before + 1, "one slider, one entry");

    let _ = app.update(Message::Undo);
    assert!(
        (motif_complexity(&app, def_id) - old_complexity).abs() < 1e-6,
        "one undo restores the pre-drag value"
    );
}

/// Switching from one chord-inspector slider to another breaks the coalesce
/// run — each knob has its own key, so this is two entries.
#[test]
fn switching_chord_inspector_slider_breaks_the_coalesce_run() {
    let mut app = app_with_redo(ViewMode::Compose);
    demo::seed_demo_content(&mut app);
    app.test_set_dirty(false);
    let def_id = app.compose_state().definitions[0].id;
    let before = entries(&app);

    let _ = app.update(chord_inspector(
        def_id,
        ChordInspectorMsg::SetMotifComplexity(0.3),
    ));
    let _ = app.update(chord_inspector(
        def_id,
        ChordInspectorMsg::SetMotifLeapChance(0.3),
    ));

    assert_eq!(entries(&app), before + 2, "different sliders, two entries");
}

fn bulk_lyrics(
    definition_id: u64,
    track_id: resonance_audio::types::TrackId,
    action: iced::widget::text_editor::Action,
) -> Message {
    lane_inspector(
        definition_id,
        track_id,
        LaneInspectorMsg::VocalBulkLyricsAction(action),
    )
}

/// Moving the cursor / selecting in the bulk-lyrics editor touches nothing
/// in the project — it must record no undo entry at all, for any number of
/// moves.
#[test]
fn bulk_lyrics_cursor_moves_are_no_edit() {
    use iced::widget::text_editor::{Action, Motion};

    let (mut app, def_id, track_id) = app_with_redo_and_vocal_lane();

    assert_no_edit(
        &mut app,
        vec![
            bulk_lyrics(def_id, track_id, Action::Move(Motion::Right)),
            bulk_lyrics(def_id, track_id, Action::Move(Motion::End)),
            bulk_lyrics(def_id, track_id, Action::Select(Motion::Left)),
            bulk_lyrics(def_id, track_id, Action::SelectAll),
            bulk_lyrics(def_id, track_id, Action::Click(iced::Point::ORIGIN)),
        ],
    );
}

/// The bulk editor's `iced::widget::text_editor::Content` is a session-only
/// mirror of the canonical `VocalParams::draft` — it isn't part of the
/// project and isn't touched by undo/redo restore (only re-synced by the
/// other paths that mutate the draft, `update/compose/lane_inspector/
/// vocal_params.rs`). So the coalesce test below reads the canonical draft,
/// not this mirror.
fn vocal_draft_text(app: &Resonance, definition_id: u64, track_id: resonance_audio::types::TrackId) -> String {
    let def = app
        .compose_state()
        .definitions
        .iter()
        .find(|d| d.id == definition_id)
        .expect("definition exists");
    match &def
        .lane_generators
        .get(&track_id)
        .expect("lane generator installed")
        .kind
    {
        LaneGeneratorKind::Vocal(p) => p
            .draft
            .iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>()
            .join("\n"),
        other => panic!("expected a Vocal lane generator, got {other:?}"),
    }
}

/// Typing a burst of characters into the bulk-lyrics editor is one undo
/// entry, and one undo restores the pre-typing draft.
#[test]
fn typing_bulk_lyrics_coalesces_into_one_entry() {
    use iced::widget::text_editor::{Action, Edit};

    let (mut app, def_id, track_id) = app_with_redo_and_vocal_lane();
    let old_draft = vocal_draft_text(&app, def_id, track_id);
    let before = entries(&app);

    for ch in ['h', 'i'] {
        let _ = app.update(bulk_lyrics(def_id, track_id, Action::Edit(Edit::Insert(ch))));
    }

    assert!(vocal_draft_text(&app, def_id, track_id).starts_with("hi"));
    assert_eq!(entries(&app), before + 1, "one typing burst, one entry");

    let _ = app.update(Message::Undo);
    assert_eq!(
        vocal_draft_text(&app, def_id, track_id),
        old_draft,
        "one undo restores the pre-typing draft"
    );
}

// ---------------------------------------------------------------------------
// FU-A10c: additional drum group and external latency sliders coalesce
// ---------------------------------------------------------------------------

fn group_cycle(app: &Resonance, group_id: u64) -> u32 {
    app.compose_state()
        .drum_patterns
        .iter()
        .flat_map(|p| p.groups.iter())
        .find(|g| g.id == group_id)
        .expect("group exists")
        .cycle
}

fn group_phase(app: &Resonance, group_id: u64) -> u32 {
    app.compose_state()
        .drum_patterns
        .iter()
        .flat_map(|p| p.groups.iter())
        .find(|g| g.id == group_id)
        .expect("group exists")
        .phase
}

fn pad_weight(app: &Resonance, group_id: u64, pad_index: usize) -> u32 {
    app.compose_state()
        .drum_patterns
        .iter()
        .flat_map(|p| p.groups.iter())
        .find(|g| g.id == group_id)
        .expect("group exists")
        .pads
        .get(pad_index)
        .expect("pad exists")
        .weight
}

fn external_latency_offset(app: &Resonance, track_id: resonance_audio::types::TrackId) -> i64 {
    app.test_external_instrument(track_id)
        .expect("external instrument exists")
        .latency_offset_samples
}

fn external_instrument(msg: ExternalInstrumentMessage) -> Message {
    Message::ExternalInstrument(msg)
}

/// Five steps of the same drum-group cycle knob is one undo entry,
/// and one undo restores the pre-drag value.
#[test]
fn drum_group_cycle_steps_coalesce_into_one_entry() {
    let mut app = app_with_redo(ViewMode::Compose);
    let group_id = group_id_of_first_pattern(&app);
    let old_cycle = group_cycle(&app, group_id);
    let before = entries(&app);

    for cycle in [4, 8, 12, 16, 20] {
        let _ = app.update(drums(DrumGroupsMessage::SetGroupCycle { group_id, cycle }));
    }

    assert_eq!(group_cycle(&app, group_id), 20);
    assert_eq!(entries(&app), before + 1, "one knob, one entry");

    let _ = app.update(Message::Undo);
    assert_eq!(
        group_cycle(&app, group_id),
        old_cycle,
        "one undo restores the pre-drag cycle"
    );
}

/// Five steps of the same drum-group phase knob is one undo entry,
/// and one undo restores the pre-drag value.
#[test]
fn drum_group_phase_steps_coalesce_into_one_entry() {
    let mut app = app_with_redo(ViewMode::Compose);
    let group_id = group_id_of_first_pattern(&app);
    let old_phase = group_phase(&app, group_id);
    let before = entries(&app);

    for phase in [1, 2, 3, 4, 5] {
        let _ = app.update(drums(DrumGroupsMessage::SetGroupPhase { group_id, phase }));
    }

    assert_eq!(group_phase(&app, group_id), 5);
    assert_eq!(entries(&app), before + 1, "one knob, one entry");

    let _ = app.update(Message::Undo);
    assert_eq!(
        group_phase(&app, group_id),
        old_phase,
        "one undo restores the pre-drag phase"
    );
}

/// Five steps of the same drum-pad weight knob is one undo entry,
/// and one undo restores the pre-drag value.
#[test]
fn drum_pad_weight_steps_coalesce_into_one_entry() {
    let mut app = app_with_redo(ViewMode::Compose);
    let group_id = group_id_of_first_pattern(&app);
    let pad_index = 0;
    let old_weight = pad_weight(&app, group_id, pad_index);
    let before = entries(&app);

    for weight in [20, 40, 60, 80, 100] {
        let _ = app.update(drums(DrumGroupsMessage::SetPadWeight {
            group_id,
            pad_index,
            weight,
        }));
    }

    assert_eq!(pad_weight(&app, group_id, pad_index), 100);
    assert_eq!(entries(&app), before + 1, "one pad, one entry");

    let _ = app.update(Message::Undo);
    assert_eq!(
        pad_weight(&app, group_id, pad_index),
        old_weight,
        "one undo restores the pre-drag weight"
    );
}

/// Five steps of the same external-latency offset knob is one undo entry,
/// and one undo restores the pre-drag value.
#[test]
fn external_latency_offset_steps_coalesce_into_one_entry() {
    let mut app = app_with_redo(ViewMode::Arrange);
    const EXT_TRACK: resonance_audio::types::TrackId = 3;
    add_external_track_with_clip(&mut app, EXT_TRACK);
    let _ = app.update(external_instrument(ExternalInstrumentMessage::Enable(EXT_TRACK)));
    app.test_set_dirty(false);
    let old_latency = external_latency_offset(&app, EXT_TRACK);
    let before = entries(&app);

    for offset in [100, 200, 300, 400, 500] {
        let _ = app.update(external_instrument(ExternalInstrumentMessage::SetLatencyOffset(
            EXT_TRACK, offset,
        )));
    }

    assert_eq!(external_latency_offset(&app, EXT_TRACK), 500);
    assert_eq!(entries(&app), before + 1, "one track, one entry");

    let _ = app.update(Message::Undo);
    assert_eq!(
        external_latency_offset(&app, EXT_TRACK),
        old_latency,
        "one undo restores the pre-drag offset"
    );
}
