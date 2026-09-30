//! The keymap in settings and Preferences › Keyboard (command-palette.md
//! §8, §9 P5): presets, rebinding by pressing the chord, conflicts that name
//! the owner, reset, persistence, and a golden of the panel.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::commands::{CommandId, KeyChord, KeymapPreset, Mods, NamedKey, Scope};
use resonance_app::message::{Message, UiMessage};
use resonance_app::settings::{KeymapOverride, KeymapSettings};
use resonance_app::state::ViewMode;
use resonance_app::update::keymap::{resolve, KeymapMsg, SettingsTab};
use resonance_app::update::shortcuts::TypingProbe;
use resonance_app::{demo, theme, Resonance};

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_typing_probe(TypingProbe::Assume { editing: false });
    app
}

fn keymap(app: &mut Resonance, m: KeymapMsg) {
    let _ = app.update(Message::Ui(UiMessage::Keymap(m)));
}

fn press(app: &mut Resonance, chord: KeyChord) {
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord,
        repeat: false,
        captured: false,
    }));
}

fn key(c: char) -> KeyChord {
    KeyChord::char(c, Mods::NONE)
}

fn keyboard_panel(app: &mut Resonance) {
    let _ = app.update(Message::Ui(UiMessage::OpenSettings));
    keymap(app, KeymapMsg::SetTab(SettingsTab::Keyboard));
}

#[test]
fn resolve_replays_overrides_onto_the_preset() {
    let settings = KeymapSettings {
        preset: "AbletonLive".into(),
        overrides: vec![
            KeymapOverride {
                command: "TransportToggleMetronome".into(),
                chord: Some("J".into()),
            },
            KeymapOverride {
                command: "NoSuchCommand".into(),
                chord: Some("X".into()),
            },
            KeymapOverride {
                command: "TransportToggleLoop".into(),
                chord: None,
            },
        ],
    };
    let map = resolve(&settings);
    assert_eq!(
        map.chord_for(CommandId::TransportRecord),
        Some(KeyChord::named(NamedKey::Enter, Mods::NONE)),
        "the Ableton preset applies"
    );
    assert_eq!(map.command_for(Scope::Global, key('j')), Some(CommandId::TransportToggleMetronome));
    assert_eq!(map.chord_for(CommandId::TransportToggleLoop), None, "None unbinds");
    // The settings round-trip through JSON.
    let json = serde_json::to_string(&settings).unwrap();
    assert_eq!(serde_json::from_str::<KeymapSettings>(&json).unwrap(), settings);
}

#[test]
fn rebinding_takes_the_next_key_press_and_it_works_straight_away() {
    let mut app = app();
    keyboard_panel(&mut app);
    keymap(&mut app, KeymapMsg::BeginRebind(CommandId::TransportToggleMetronome));
    press(&mut app, key('j'));
    assert_eq!(
        app.test_keymap().command_for(Scope::Global, key('j')),
        Some(CommandId::TransportToggleMetronome)
    );
    assert_eq!(app.test_keymap().command_for(Scope::Global, key('k')), None);
    assert_eq!(app.test_settings().keymap.overrides.len(), 1, "persisted as an override");

    let _ = app.update(Message::Ui(UiMessage::CloseSettings));
    let before = format!("{:?}", app.test_settings().keymap);
    press(&mut app, key('j'));
    assert_eq!(format!("{:?}", app.test_settings().keymap), before);
    assert!(
        app.test_settings().palette.recent.first().is_some_and(|k| k == "TransportToggleMetronome"),
        "J now runs the metronome toggle"
    );
}

#[test]
fn a_taken_chord_asks_first_and_names_the_owner() {
    let mut app = app();
    keyboard_panel(&mut app);
    keymap(&mut app, KeymapMsg::BeginRebind(CommandId::TransportToggleMetronome));
    press(&mut app, key('l'));
    let conflict = app.test_keymap_editor().conflict.expect("a conflict");
    assert_eq!(conflict.owner, CommandId::TransportToggleLoop);
    assert_eq!(
        app.test_keymap().command_for(Scope::Global, key('l')),
        Some(CommandId::TransportToggleLoop),
        "nothing changes until confirmed"
    );
    keymap(&mut app, KeymapMsg::ConfirmConflict);
    assert_eq!(
        app.test_keymap().command_for(Scope::Global, key('l')),
        Some(CommandId::TransportToggleMetronome)
    );
    assert_eq!(app.test_keymap().chord_for(CommandId::TransportToggleLoop), None);

    keymap(&mut app, KeymapMsg::ResetAll);
    assert_eq!(
        app.test_keymap().command_for(Scope::Global, key('l')),
        Some(CommandId::TransportToggleLoop)
    );
}

#[test]
fn esc_while_listening_cancels_the_rebind_not_the_dialog() {
    let mut app = app();
    keyboard_panel(&mut app);
    keymap(&mut app, KeymapMsg::BeginRebind(CommandId::TransportToggleLoop));
    press(&mut app, KeyChord::named(NamedKey::Escape, Mods::NONE));
    assert!(app.test_keymap_editor().capturing.is_none());
    assert!(app.root_overlay().is_some(), "Settings stays open");
    assert_eq!(
        app.test_keymap().chord_for(CommandId::TransportToggleLoop),
        Some(key('l'))
    );
}

#[test]
fn reset_restores_one_command_and_presets_switch_the_table() {
    let mut app = app();
    keyboard_panel(&mut app);
    keymap(&mut app, KeymapMsg::BeginRebind(CommandId::TransportToggleLoop));
    press(&mut app, key('j'));
    keymap(&mut app, KeymapMsg::Reset(CommandId::TransportToggleLoop));
    assert_eq!(app.test_keymap().chord_for(CommandId::TransportToggleLoop), Some(key('l')));

    keymap(&mut app, KeymapMsg::SetPreset(KeymapPreset::AbletonLive));
    assert_eq!(app.test_settings().keymap.preset, "AbletonLive");
    assert_eq!(
        app.test_keymap().chord_for(CommandId::TransportRecord),
        Some(KeyChord::named(NamedKey::Enter, Mods::NONE))
    );
}

fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(theme::ICON_FONT_BYTES.into());
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    }
}

/// The Keyboard page: one edited row, one conflict banner.
#[test]
fn keyboard_panel_golden() {
    let mut app = app();
    demo::seed_demo_content(&mut app);
    keyboard_panel(&mut app);
    keymap(&mut app, KeymapMsg::BeginRebind(CommandId::TransportTogglePlay));
    press(&mut app, key('j'));
    keymap(&mut app, KeymapMsg::BeginRebind(CommandId::TransportPlayPause));
    press(&mut app, key('l'));
    assert!(app.test_keymap_editor().conflict.is_some());
    let mut ui = Simulator::with_size(sim_settings(), Size::new(1440.0, 900.0), app.view());
    let snap = ui.snapshot(&theme::resonance_theme()).expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/keymap_settings_panel.png");
}
