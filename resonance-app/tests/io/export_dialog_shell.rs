//! Golden-image snapshots for the **Export modal** (design doc #155,
//! todo #324; body wired by code review ARCH2-01).
//!
//! Two states, both reached through the real `ExportMessage` reducer:
//!
//! 1. **Audio stems tab (default)** — the source checklist (master,
//!    busses, top-level tracks), the range toggle and the destination
//!    row; "0 selected" and the primary action disabled.
//! 2. **MIDI tab** — the accent moves to the MIDI tab, whose body notes
//!    that MIDI export is not available yet.
//!
//! The render itself (`ExportStems` and its events) is covered by
//! `export_stems.rs`.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{ExportMessage, Message};
use resonance_app::state::{ExportMode, ViewMode};
use resonance_app::{demo, theme, Resonance};

/// Window size matches the app's default & minimum window per the
/// design guidelines, same as the other `iced_test` snapshots.
const WINDOW: (f32, f32) = (1440.0, 900.0);

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

/// Demo app pinned to the Compose tab with the Export modal opened
/// through the real `ExportMessage::Open` reducer, so each snapshot has
/// representative content dimmed behind the overlay.
fn build_app_with_export_open() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Export(ExportMessage::Open));
    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// Freshly-opened modal: Audio-stems tab active (accent border), the
/// placeholder body, and the footer showing "0 selected" with a disabled
/// "Export stems" action.
#[test]
fn export_dialog_audio_stems_tab() {
    let app = build_app_with_export_open();
    assert_eq!(
        app.test_export_dialog().map(|d| d.mode),
        Some(ExportMode::AudioStems),
        "Open should show the modal in Audio-stems mode",
    );
    snapshot_to(&app, "tests/snapshots/export_dialog_audio_stems_tab.png");
}

/// Switching to the MIDI tab through the real `SetMode` reducer moves the
/// accent to the MIDI tab and swaps the body hint + primary label to the
/// MIDI copy — proving the shell's tab switch is wired.
#[test]
fn export_dialog_midi_tab() {
    let mut app = build_app_with_export_open();
    let _ = app.update(Message::Export(ExportMessage::SetMode(ExportMode::Midi)));
    assert_eq!(
        app.test_export_dialog().map(|d| d.mode),
        Some(ExportMode::Midi),
        "SetMode(Midi) should switch the active tab",
    );
    snapshot_to(&app, "tests/snapshots/export_dialog_midi_tab.png");
}

/// Close plumbing: `ExportMessage::Close` tears the overlay down, and the
/// disabled-action invariant holds while no source is selected (selection
/// lands with the per-tab bodies in #326/#327).
#[test]
fn export_dialog_open_close_plumbing() {
    let mut app = build_app_with_export_open();
    let dialog = app.test_export_dialog().expect("modal open after Open");
    assert_eq!(dialog.selected_count(), 0, "fresh dialog selects nothing");
    assert!(!dialog.can_export(), "primary action disabled with no sources");

    let _ = app.update(Message::Export(ExportMessage::Close));
    assert!(
        app.test_export_dialog().is_none(),
        "Close should tear the modal down",
    );
}
