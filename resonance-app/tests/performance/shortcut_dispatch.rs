//! The registry key dispatcher end to end (command-palette.md §3.2, §4.1,
//! §10 P0): the overlay gate, Esc resolution, the key-repeat filter and the
//! typing gate, all driven through `UiMessage::ShortcutKey` exactly as the
//! keyboard subscription sends it.

use resonance_app::commands::{KeyChord, Mods, NamedKey};
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::{Overlay, ViewMode};
use resonance_app::update::shortcuts::TypingProbe;
use resonance_app::Resonance;

/// A project-open app whose typing gate answers "nothing is focused".
pub(crate) fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_typing_probe(TypingProbe::Assume { editing: false });
    app
}

pub(crate) fn press(app: &mut Resonance, chord: KeyChord) {
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord,
        repeat: false,
        captured: false,
    }));
}

pub(crate) fn repeat(app: &mut Resonance, chord: KeyChord) {
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord,
        repeat: true,
        captured: false,
    }));
}

pub(crate) fn key(c: char) -> KeyChord {
    KeyChord::char(c, Mods::NONE)
}

pub(crate) fn esc() -> KeyChord {
    KeyChord::named(NamedKey::Escape, Mods::NONE)
}

fn project_dir(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "resonance_shortcut_dispatch_{tag}_{}",
        std::process::id()
    ));
    std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
    let dir = root.join("song.rproj");
    std::fs::create_dir_all(&dir).expect("create project dir");
    dir
}

/// With Settings open, bare keys dispatch nothing, a ⌘ chord still runs
/// and Esc closes Settings (the "F under Settings" bug).
#[test]
fn a_modal_overlay_blocks_bare_keys_but_not_cmd_chords_or_esc() {
    let mut app = app();
    app.test_set_project_path(project_dir("overlay_gate"));
    let _ = app.update(Message::Ui(UiMessage::OpenSettings));
    assert_eq!(app.root_overlay(), Some(Overlay::Settings));

    press(&mut app, key('f'));
    assert_eq!(app.test_view_mode(), ViewMode::Arrange, "F must not toggle under Settings");
    let loop_before = app.test_loop_range();
    press(&mut app, key('l'));
    assert_eq!(app.test_loop_range(), loop_before, "L must not toggle the loop under Settings");

    press(&mut app, KeyChord::char('s', Mods::cmd()));
    assert!(app.test_save_in_flight().is_some(), "⌘S still saves under Settings");
    assert_eq!(app.root_overlay(), Some(Overlay::Settings));

    press(&mut app, esc());
    assert_eq!(app.root_overlay(), None, "Esc closes Settings");
    assert_eq!(app.test_view_mode(), ViewMode::Arrange);
}

/// Esc closes the overlay first and only then means "exit Performance".
#[test]
fn esc_closes_the_overlay_before_it_exits_performance_mode() {
    let mut app = app();
    press(&mut app, key('f'));
    assert_eq!(app.test_view_mode(), ViewMode::Performance);
    let _ = app.update(Message::Ui(UiMessage::OpenAddTrackMenu));
    assert_eq!(app.root_overlay(), Some(Overlay::AddTrackMenu));

    press(&mut app, esc());
    assert_eq!(app.root_overlay(), None);
    assert_eq!(app.test_view_mode(), ViewMode::Performance, "first Esc only closes the menu");

    press(&mut app, esc());
    assert_eq!(app.test_view_mode(), ViewMode::Arrange);
}

/// Holding F toggles Performance mode exactly once.
#[test]
fn a_held_f_toggles_performance_mode_once() {
    let mut app = app();
    press(&mut app, key('f'));
    for _ in 0..5 {
        repeat(&mut app, key('f'));
    }
    assert_eq!(app.test_view_mode(), ViewMode::Performance);
}

/// A key a widget already consumed (a focused text field, a canvas that
/// owns the keys) never reaches the registry.
#[test]
fn a_captured_key_dispatches_nothing() {
    let mut app = app();
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: key('f'),
        repeat: false,
        captured: true,
    }));
    assert_eq!(app.test_view_mode(), ViewMode::Arrange);
}

/// A bare key is dropped while a text field holds focus.
#[test]
fn a_bare_key_is_dropped_while_typing() {
    let mut app = app();
    app.test_set_typing_probe(TypingProbe::Assume { editing: true });
    press(&mut app, key('f'));
    assert_eq!(app.test_view_mode(), ViewMode::Arrange);
}

/// The overlay the gate reads is the overlay the view draws: the startup
/// screen, which Esc must not close.
#[test]
fn the_startup_screen_is_a_modal_overlay_esc_cannot_close() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_typing_probe(TypingProbe::Assume { editing: false });
    assert_eq!(app.root_overlay(), Some(Overlay::Startup));
    press(&mut app, esc());
    assert_eq!(app.root_overlay(), Some(Overlay::Startup));
    press(&mut app, key('f'));
    assert_eq!(app.test_view_mode(), ViewMode::Arrange);
}
