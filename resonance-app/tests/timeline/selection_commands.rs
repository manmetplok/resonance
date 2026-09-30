//! Selection commands and orphan entry points (command-palette.md §3.4,
//! §4.3, §5.3–5.6, §10 P3): each command lands as exactly one undo entry,
//! mixed selections resolve to "all on", and the actions that had no GUI
//! entry point reach their modal or reducer.

use resonance_app::commands::{BindingMap, CommandId, KeyChord, Mods, NamedKey, Scope};
use resonance_app::message::{Message, TransportMessage, UiMessage};
use resonance_app::state::{ArrangementMarker, ClipState, ViewMode};
use resonance_app::update::shortcuts::TypingProbe;
use resonance_app::Resonance;
use resonance_audio::types::{FadeCurve, TrackType};

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_sample_rate(48_000);
    app.test_set_typing_probe(TypingProbe::Assume { editing: false });
    // Undo records only for a project with a path.
    app.test_set_project_path(std::env::temp_dir().join("resonance_selection_commands.rproj"));
    app
}

fn run(app: &mut Resonance, command: CommandId) {
    let _ = app.update(Message::Ui(UiMessage::RunShortcut(command)));
}

fn undo_len(app: &Resonance) -> usize {
    app.test_undo_history().undo_len()
}

fn clip(id: u64, start: u64, len: u64) -> ClipState {
    ClipState {
        id,
        track_id: 1,
        start_sample: start,
        duration_samples: len,
        name: "clip".into(),
        total_frames: len,
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
    }
}

fn three_selected_tracks() -> Resonance {
    let mut app = app();
    for id in 1..=3 {
        app.test_add_track(id, TrackType::Audio);
    }
    app.test_set_selected_tracks(vec![1, 2, 3]);
    app
}

fn flags(app: &Resonance, f: impl Fn(&resonance_app::state::TrackState) -> bool) -> Vec<bool> {
    app.test_registry().tracks.iter().map(f).collect()
}

#[test]
fn mute_solo_and_arm_selected_are_one_undo_entry_each_and_mixed_means_all_on() {
    let mut app = three_selected_tracks();
    let _ = app.update(Message::Track(resonance_app::message::TrackMessage::ToggleMute(2)));
    assert_eq!(flags(&app, |t| t.muted), [false, true, false]);

    let before = undo_len(&app);
    run(&mut app, CommandId::ToggleMuteSelected);
    assert_eq!(undo_len(&app), before + 1, "one entry for three tracks");
    assert_eq!(flags(&app, |t| t.muted), [true, true, true], "mixed → all on");
    run(&mut app, CommandId::ToggleMuteSelected);
    assert_eq!(flags(&app, |t| t.muted), [false, false, false], "all on → all off");

    let before = undo_len(&app);
    run(&mut app, CommandId::ToggleSoloSelected);
    assert_eq!(undo_len(&app), before + 1);
    assert_eq!(flags(&app, |t| t.soloed), [true, true, true]);

    let before = undo_len(&app);
    run(&mut app, CommandId::ToggleArmSelected);
    assert_eq!(undo_len(&app), before + 1);
    assert_eq!(flags(&app, |t| t.record_armed), [true, true, true]);

    let _ = app.update(Message::Undo);
    assert_eq!(flags(&app, |t| t.record_armed), [false, false, false], "undo takes all three back");
}

#[test]
fn split_at_playhead_cuts_the_selected_clip_in_one_undo_entry() {
    let mut app = app();
    app.test_add_track(1, TrackType::Audio);
    app.test_push_clip(clip(7, 0, 48_000));
    app.test_set_selected_clip(Some(7));
    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(20_000)));

    let before = undo_len(&app);
    run(&mut app, CommandId::SplitClipAtPlayhead);
    assert_eq!(undo_len(&app), before + 1);
    let mut spans: Vec<(u64, u64)> = app
        .test_clips()
        .iter()
        .map(|c| (c.start_sample, c.duration_samples))
        .collect();
    spans.sort();
    assert_eq!(spans, [(0, 20_000), (20_000, 28_000)]);

    // Outside the clip there is nothing to cut.
    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(90_000)));
    assert!(!CommandId::SplitClipAtPlayhead.availability(&app).is_yes());
}

#[test]
fn loop_selection_loops_the_selected_clip() {
    let mut app = app();
    app.test_add_track(1, TrackType::Audio);
    app.test_push_clip(clip(7, 10_000, 30_000));
    assert!(!CommandId::LoopSelection.availability(&app).is_yes());
    app.test_set_selected_clip(Some(7));
    run(&mut app, CommandId::LoopSelection);
    assert_eq!(app.test_loop_range(), (10_000, 40_000, true));
}

#[test]
fn loop_selection_falls_back_to_the_selected_marker_region() {
    let mut app = app();
    let mut region = ArrangementMarker::new_point(1, "Chorus".into(), [0; 3], 48_000);
    region.end_sample = Some(96_000);
    let id = app.test_add_marker(region);
    let _ = app.update(Message::MarkerUi(resonance_app::message::MarkerUiMessage::Select(Some(id))));
    run(&mut app, CommandId::LoopSelection);
    assert_eq!(app.test_loop_range(), (48_000, 96_000, true));
}

#[test]
fn orphan_actions_reach_their_modal_or_reducer() {
    let mut app = app();
    run(&mut app, CommandId::ExportStemsMidi);
    assert!(app.test_export_dialog().is_some(), "Export modal opens");
    let _ = app.update(Message::Ui(UiMessage::DismissOverlay));
    assert!(app.test_export_dialog().is_none(), "Esc's dismiss closes it");

    run(&mut app, CommandId::ImportMidi);
    assert!(app.test_import_dialog().is_some(), "Import MIDI modal opens");
    let _ = app.update(Message::Ui(UiMessage::DismissOverlay));

    let markers = app.test_markers().len();
    run(&mut app, CommandId::AddMarkerAtPlayhead);
    assert_eq!(app.test_markers().len(), markers + 1);

    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(1_000)));
    let chords = app.test_chord_track().regions.len();
    run(&mut app, CommandId::AddChordAtPlayhead);
    assert_eq!(app.test_chord_track().regions.len(), chords + 1);
    run(&mut app, CommandId::ToggleChordPinAtPlayhead);
    assert!(app.test_chord_track().regions.iter().any(|r| r.pinned));
    run(&mut app, CommandId::DeleteChordAtPlayhead);
    assert_eq!(app.test_chord_track().regions.len(), chords);

    // Save as Template names itself after the project.
    let message = CommandId::SaveAsTemplate.to_message(&app).expect("a message");
    assert!(
        format!("{message:?}").contains("resonance_selection_commands"),
        "{message:?}"
    );

    let before = undo_len(&app);
    run(&mut app, CommandId::AddDrumTrack);
    assert_eq!(undo_len(&app), before + 1, "the drum-track add is one undoable step");
}

#[test]
fn a_canvas_command_runs_from_the_palette_against_the_selection() {
    let mut app = app();
    app.test_add_track(1, TrackType::Audio);
    app.test_push_clip(clip(7, 0, 48_000));
    assert!(!CommandId::TimelineDeleteSelection.availability(&app).is_yes());
    app.test_set_selected_clip(Some(7));
    run(&mut app, CommandId::TimelineDeleteSelection);
    assert!(app.test_clips().is_empty());
}

/// §4.3: a canvas-scoped chord may shadow a global one — the lookup is per
/// scope, so both meanings coexist.
#[test]
fn canvas_scopes_shadow_global_chords() {
    let map = BindingMap::resonance_default();
    let s = KeyChord::char('s', Mods::NONE);
    assert_eq!(map.command_for(Scope::Global, s), Some(CommandId::ToggleSoloSelected));
    assert_eq!(map.command_for(Scope::VocalRoll, s), Some(CommandId::VocalToggleSlur));
    let esc = KeyChord::named(NamedKey::Escape, Mods::NONE);
    assert_eq!(map.command_for(Scope::Global, esc), Some(CommandId::ExitPerformanceMode));
    assert_eq!(map.command_for(Scope::ExpandedEditor, esc), Some(CommandId::ComposeCollapseTrack));
    let del = KeyChord::named(NamedKey::Delete, Mods::NONE);
    assert_eq!(map.command_for(Scope::Global, del), None);
    assert_eq!(map.command_for(Scope::Timeline, del), Some(CommandId::TimelineDeleteSelection));
}

#[test]
fn duplicate_places_the_selected_section_right_after_it() {
    let mut app = app();
    app.test_push_section_definition(crate::transport_control::section_def(1, 4));
    let placement = app.test_place_section(1, 2);
    let _ = app.update(Message::Compose(
        resonance_app::compose::ComposeMessage::SelectSectionPlacement { placement_id: placement },
    ));
    let before = undo_len(&app);
    run(&mut app, CommandId::DuplicateSelection);
    assert_eq!(undo_len(&app), before + 1);
    let mut bars: Vec<u32> = app.test_placements().iter().map(|p| p.2).collect();
    bars.sort();
    assert_eq!(bars, [2, 6]);
}
