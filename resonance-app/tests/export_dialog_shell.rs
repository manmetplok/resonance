//! Golden-image snapshots for the **Export modal shared shell**
//! (design doc #155, todo #324).
//!
//! The Export modal is a single overlay with two mode tabs — Audio
//! stems / MIDI — sharing one source selection and a footer (live count
//! + primary action). This scaffold todo builds only the shell: the
//! dimmed backdrop, the centered BG_2 container, the serif-italic title,
//! the mode tabs, and the footer. The per-tab body widgets (#326/#327)
//! and the render phases (#328) are follow-ups, so the body is a
//! placeholder hint keyed off the active mode.
//!
//! Two states are locked in here — both reachable through the real
//! `ExportMessage` reducer path:
//!
//! 1. **Audio stems tab (default)** — the modal as it opens: Audio-stems
//!    tab active with the accent border, "0 selected" in the footer, and
//!    the primary action disabled (no source ticked yet).
//! 2. **MIDI tab** — after `ExportMessage::SetMode(Midi)` the accent moves
//!    to the MIDI tab and the body hint + primary label switch to the
//!    MIDI copy.
//!
//! Source selection (which would enable the primary action) lands with
//! the per-tab bodies in #326/#327, so it isn't reachable yet — both
//! snapshots show the empty-selection footer.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{ExportMessage, Message};
use resonance_app::state::{ExportMode, ViewMode};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

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
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Export(ExportMessage::Open));
    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
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
