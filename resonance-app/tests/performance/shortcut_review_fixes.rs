//! Regressions from the command-palette review: each test pins one fix.

use iced::keyboard::{self, Key, Modifiers};
use iced::Point;
use resonance_app::commands::{CommandId, KeyChord, Mods, NamedKey};
use resonance_app::message::{Message, TrackMessage, TransportMessage, UiMessage};
use resonance_app::palette::{PaletteItem, PaletteMode, PaletteMsg};
use resonance_app::settings::AppSettings;
use resonance_app::state::{FreezeStatus, Overlay, ViewMode};
use resonance_app::update::shortcuts::TypingProbe;
use resonance_app::Resonance;
use resonance_audio::types::TrackType;
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_sample_rate(48_000);
    app.test_set_typing_probe(TypingProbe::Assume { editing: false });
    app
}

fn press(app: &mut Resonance, chord: KeyChord) {
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord,
        repeat: false,
        captured: false,
    }));
}

fn palette(app: &mut Resonance, m: PaletteMsg) {
    let _ = app.update(Message::Ui(UiMessage::Palette(m)));
}

fn recent(app: &Resonance) -> Vec<String> {
    app.test_settings().palette.recent.clone()
}

// --- New Project -----------------------------------------------------------

#[test]
fn new_project_replaces_the_open_project_instead_of_saving_it_elsewhere() {
    let mut app = app();
    app.test_set_project_path(std::env::temp_dir().join("resonance_review_new.rproj"));
    app.test_run_shortcut(CommandId::NewProject);
    assert!(app.test_has_active_project());
    assert_eq!(app.test_project_path(), None, "a fresh untitled project, not Save As");
}

#[test]
fn new_project_is_unavailable_with_unsaved_changes() {
    let mut app = app();
    app.test_set_project_path(std::env::temp_dir().join("resonance_review_dirty.rproj"));
    app.test_set_dirty(true);
    assert!(!CommandId::NewProject.availability(&app).is_yes());
    app.test_run_shortcut(CommandId::NewProject);
    assert!(app.test_project_path().is_some(), "the dirty project stays open");
}

// --- Recents ---------------------------------------------------------------

#[test]
fn opening_the_palette_is_not_a_recent_command() {
    let mut app = app();
    press(&mut app, KeyChord::char('k', Mods::cmd()));
    assert!(app.test_palette().is_some());
    press(&mut app, KeyChord::char('k', Mods::cmd()));
    press(&mut app, KeyChord::char('j', Mods::cmd()));
    assert!(recent(&app).is_empty(), "{:?}", recent(&app));
}

#[test]
fn a_command_a_gate_refuses_is_not_recorded() {
    // No project open: the startup gate swallows transport messages.
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_typing_probe(TypingProbe::Assume { editing: false });
    app.test_run_shortcut(CommandId::TransportToggleMetronome);
    assert!(recent(&app).is_empty());
}

// --- Arm ---------------------------------------------------------------------

#[test]
fn arm_selected_skips_frozen_tracks() {
    let mut app = app();
    app.test_add_track(1, TrackType::Audio);
    app.test_add_track(2, TrackType::Audio);
    let cache = FreezeCacheRef::new("f.wav".to_owned(), 48_000, 32, 1, FreezeCacheStatus::Frozen);
    app.test_set_freeze_status(2, FreezeStatus::Frozen { cache_ref: cache });
    app.test_set_selected_tracks(vec![1, 2]);
    app.test_run_shortcut(CommandId::ToggleArmSelected);
    let armed: Vec<bool> = app.test_registry().tracks.iter().map(|t| t.record_armed).collect();
    assert_eq!(armed, [true, false]);

    app.test_set_selected_tracks(vec![2]);
    assert!(!CommandId::ToggleArmSelected.availability(&app).is_yes());
}

// --- Palette hover -----------------------------------------------------------

#[test]
fn a_resting_pointer_never_steals_the_keyboard_selection() {
    let mut app = app();
    let _ = app.update(Message::Ui(UiMessage::OpenPalette(PaletteMode::Commands)));
    palette(&mut app, PaletteMsg::Query("toggle".into()));
    // The card appears under a resting pointer: its first report is not a move.
    palette(&mut app, PaletteMsg::PointerMoved(Point::new(100.0, 200.0)));
    palette(&mut app, PaletteMsg::Hover(3));
    assert_eq!(app.test_palette().unwrap().selected, 0);
    // A real move arms hover.
    palette(&mut app, PaletteMsg::PointerMoved(Point::new(101.0, 200.0)));
    palette(&mut app, PaletteMsg::Hover(3));
    assert_eq!(app.test_palette().unwrap().selected, 3);
    // ↓ scrolls rows under the resting pointer: hover is disarmed again.
    palette(&mut app, PaletteMsg::Move(1));
    palette(&mut app, PaletteMsg::Hover(1));
    assert_eq!(app.test_palette().unwrap().selected, 4);
}

// --- Track menu overlay --------------------------------------------------------

#[test]
fn the_track_menu_gates_bare_keys_and_esc_closes_it() {
    let mut app = app();
    app.test_add_track(1, TrackType::Audio);
    let _ = app.update(Message::Ui(UiMessage::OpenTrackMenu { id: 1, x: 10.0, y: 10.0 }));
    assert_eq!(app.root_overlay(), Some(Overlay::TrackMenu));
    press(&mut app, KeyChord::char('m', Mods::NONE));
    assert!(!app.test_registry().tracks[0].muted, "M under the menu must not mute");
    press(&mut app, KeyChord::named(NamedKey::Escape, Mods::NONE));
    assert!(app.test_track_menu().is_none());

    let _ = app.update(Message::Track(TrackMessage::OpenSavePresetPrompt(1)));
    assert_eq!(app.root_overlay(), Some(Overlay::TrackMenu));
    press(&mut app, KeyChord::named(NamedKey::Escape, Mods::NONE));
    assert_eq!(app.root_overlay(), None, "Esc closes the preset prompt");
}

// --- Hold-B audition -------------------------------------------------------------

#[test]
fn cmd_b_is_not_the_reference_audition() {
    let press_b = |modifiers| keyboard::Event::KeyPressed {
        key: Key::Character("b".into()),
        modified_key: Key::Character("b".into()),
        physical_key: keyboard::key::Physical::Code(keyboard::key::Code::KeyB),
        location: keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    };
    use resonance_app::update::momentary_audition_message;
    assert!(momentary_audition_message(press_b(Modifiers::COMMAND)).is_none());
    assert!(momentary_audition_message(press_b(Modifiers::COMMAND | Modifiers::ALT)).is_none());
    assert!(momentary_audition_message(press_b(Modifiers::empty())).is_some());
}

// --- Chords ------------------------------------------------------------------------

#[cfg(not(target_os = "macos"))]
#[test]
fn super_chords_bind_nothing_and_ctrl_parses_as_the_accelerator() {
    assert_eq!(
        KeyChord::from_iced(&Key::Character("s".into()), Modifiers::LOGO),
        None,
        "Super+S must not solo"
    );
    assert_eq!(KeyChord::parse("Ctrl+S"), Some(KeyChord::char('s', Mods::cmd())));
}

// --- Recents write ------------------------------------------------------------------

#[test]
fn recents_are_written_after_a_quiet_spell_not_per_keypress() {
    let mut app = app();
    app.test_run_shortcut(CommandId::TransportToggleMetronome);
    app.test_run_shortcut(CommandId::TransportToggleLoop);
    assert!(app.test_recent_write_pending(), "the write waits");
    let _ = app.update(Message::Tick);
    assert!(app.test_recent_write_pending(), "still within the quiet spell");
}

fn section_def(id: u64, length_bars: u32) -> resonance_app::compose::SectionDefinitionState {
    resonance_app::compose::SectionDefinitionState {
        id,
        name: format!("S{id}"),
        color: [0, 0, 0],
        length_bars,
        chords: Vec::new(),
        scale: None,
        progression_seed: 0,
        generate_params: resonance_app::compose::GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: std::collections::HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: resonance_music_theory::MotifSource::default(),
        arrangement: Vec::new(),
    }
}

// --- PlayFromLoopStart ------------------------------------------------------------------

fn loop_at(app: &mut Resonance, loop_in: u64, loop_out: u64) {
    let _ = app.update(Message::Transport(TransportMessage::SetLoopRange {
        loop_in,
        loop_out,
        enabled: None,
    }));
}

#[test]
fn play_from_loop_start_while_playing_moves_play_start() {
    let mut app = app();
    loop_at(&mut app, 48_000, 96_000);
    let _ = app.update(Message::Transport(TransportMessage::Play));
    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(70_000)));
    let _ = app.update(Message::Transport(TransportMessage::PlayFromLoopStart));
    let _ = app.update(Message::Transport(TransportMessage::TogglePlay));
    assert_eq!(app.test_playhead(), 48_000, "Space returns to the loop start");
}

#[test]
fn play_from_loop_start_wins_over_the_compose_section_auto_loop() {
    let mut app = app();
    app.test_set_view_mode(ViewMode::Compose);
    app.test_push_section_definition(section_def(1, 4));
    let placement = app.test_place_section(1, 8);
    let _ = app.update(Message::Compose(
        resonance_app::compose::ComposeMessage::SelectSectionPlacement { placement_id: placement },
    ));
    loop_at(&mut app, 48_000, 96_000);
    let _ = app.update(Message::Transport(TransportMessage::PlayFromLoopStart));
    assert!(app.test_transport_playing());
    assert_eq!(app.test_playhead(), 48_000);
    assert_eq!(app.test_loop_range().0, 48_000, "the section didn't take the loop");
}

// --- Recording ------------------------------------------------------------------------

#[test]
fn seeks_are_unavailable_while_recording() {
    let mut app = app();
    loop_at(&mut app, 48_000, 96_000);
    app.test_set_transport_recording(true);
    for id in [
        CommandId::NudgeForwardBar,
        CommandId::PlayheadToLoopStart,
        CommandId::PlayheadToStart,
        CommandId::NextSectionStart,
    ] {
        assert_eq!(
            id.availability(&app),
            resonance_app::commands::Available::No("Recording"),
            "{id:?}"
        );
    }
}

// --- Settings -----------------------------------------------------------------------------

#[test]
fn a_corrupt_keymap_entry_keeps_every_other_setting() {
    let json = r#"{
        "autosave": {"enabled": false, "interval_secs": 90},
        "keymap": {"preset": "LogicPro", "overrides": [
            {"command": "TransportToggleLoop", "chord": "J"},
            {"command": 42},
            "garbage"
        ]},
        "palette": {"recent": 7}
    }"#;
    let settings: AppSettings = serde_json::from_str(json).expect("settings survive");
    assert_eq!(settings.autosave.interval_secs, 90);
    assert!(!settings.autosave.enabled);
    assert_eq!(settings.keymap.preset, "LogicPro");
    assert_eq!(settings.keymap.overrides.len(), 1, "the good override is kept");
    assert!(settings.palette.recent.is_empty());
}

// --- Palette staleness and argument modes -------------------------------------------------

#[test]
fn a_row_that_went_unavailable_is_resolved_again_on_run() {
    let mut app = app();
    loop_at(&mut app, 48_000, 96_000);
    let _ = app.update(Message::Ui(UiMessage::OpenPalette(PaletteMode::Commands)));
    palette(&mut app, PaletteMsg::Query("playhead to loop start".into()));
    assert!(app.test_palette().unwrap().selected_row().unwrap().unavailable.is_none());
    app.test_set_transport_recording(true);
    palette(&mut app, PaletteMsg::Submit);
    let state = app.test_palette().expect("still open");
    assert_eq!(state.flash, Some("Recording"));
}

fn goto_row(app: &mut Resonance, q: &str) -> resonance_app::palette::PaletteRow {
    palette(app, PaletteMsg::Query(q.into()));
    app.test_palette().unwrap().selected_row().unwrap().clone()
}

#[test]
fn go_to_bar_validates_the_beat_relative_moves_and_the_song_end() {
    let mut app = app();
    let _ = app.update(Message::Ui(UiMessage::OpenPalette(PaletteMode::GoToBar)));
    assert!(goto_row(&mut app, ":17.9").unavailable.is_some(), "4/4 has no beat 9");
    assert!(goto_row(&mut app, ":17.4").unavailable.is_none());
    assert_eq!(goto_row(&mut app, ":+5").item, PaletteItem::Nudge(5));
    assert_eq!(goto_row(&mut app, ":-2").item, PaletteItem::Nudge(-2));

    use resonance_app::state::ArrangementMarker;
    let end = app.test_tempo_map().bar_to_sample(8);
    app.test_add_marker(ArrangementMarker::new_point(1, "End".into(), [0; 3], end));
    assert!(goto_row(&mut app, ":30").unavailable.is_some(), "past the end");
    assert!(goto_row(&mut app, ":9").unavailable.is_none());
}

#[test]
fn each_argument_mode_words_its_own_empty_state() {
    use resonance_app::palette::empty_message;
    assert_eq!(empty_message("@"), "No markers or sections");
    assert!(empty_message("#drm").starts_with("No tracks match"));
    assert!(empty_message("zz").starts_with("No commands match"));
}

#[test]
fn a_keyword_hit_keeps_the_name_highlight() {
    let (score, ranges) = resonance_app::palette::score("loop", CommandId::TransportToggleLoop).unwrap();
    assert!(score > 0);
    assert!(!ranges.is_empty(), "the name matched, so it stays highlighted");
}

#[test]
fn cmd_j_while_open_switches_to_go_to_bar() {
    let mut app = app();
    press(&mut app, KeyChord::char('k', Mods::cmd()));
    palette(&mut app, PaletteMsg::Query("sav".into()));
    press(&mut app, KeyChord::char('j', Mods::cmd()));
    assert_eq!(app.test_palette().expect("still open").query, ":");
}

// --- Keyboard panel ------------------------------------------------------------------------

#[test]
fn rebinding_keeps_the_alternates() {
    use resonance_app::update::keymap::{KeymapMsg, SettingsTab};
    let mut app = app();
    let _ = app.update(Message::Ui(UiMessage::OpenSettings));
    let _ = app.update(Message::Ui(UiMessage::Keymap(KeymapMsg::SetTab(SettingsTab::Keyboard))));
    let _ = app.update(Message::Ui(UiMessage::Keymap(KeymapMsg::BeginRebind(CommandId::Redo))));
    press(&mut app, KeyChord::char('r', Mods::cmd()));
    let chords: Vec<KeyChord> = app.test_keymap().chords_for(CommandId::Redo).collect();
    assert_eq!(chords, [KeyChord::char('r', Mods::cmd()), KeyChord::char('y', Mods::cmd())]);
}
