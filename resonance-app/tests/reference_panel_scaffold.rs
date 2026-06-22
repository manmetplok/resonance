//! Golden-image snapshot for the **Reference & A/B panel scaffold**
//! (todo #698 / design doc #184/#198).
//!
//! Covers the open/empty state: the chrome "REF" toggle opens the 360px
//! right-rail in the Mix view, and with no references loaded the panel
//! routes to its **Empty** body — the drop zone, format chips, and the
//! "Add reference…" button. A text selector locks the panel's presence
//! independently of pixels, then a golden snapshot locks its layout.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

/// Default & minimum window size per the design guidelines, matching the
/// other `iced_test` integration tests in this crate.
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

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

#[test]
fn reference_panel_open_empty() {
    // Pin the startup tab to Mixer so `view()` lands on `view_mixer`,
    // where the reference rail and its chrome toggle live.
    let _ = STARTUP_TAB.set(ViewMode::Mixer);

    let (mut app, _task) = Resonance::new();
    demo::seed_minimal_drum_track_no_busses(&mut app);

    // Belt-and-braces in case another test in this binary already set the
    // OnceLock to a different tab.
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));

    // The rail is hidden until the chrome "REF" toggle is pressed.
    let _ = app.update(Message::Ui(UiMessage::ToggleReferencePanel));

    // The empty panel must surface its title and "Add reference…" button.
    let mut ui = simulator(&app);
    ui.find("REFERENCE & A/B")
        .expect("open reference panel shows its title");
    ui.find("Add reference\u{2026}")
        .expect("empty reference panel shows the add button");

    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image("tests/snapshots/reference_panel_open_empty.png")
            .expect("matches_image i/o"),
        "snapshot diverged from golden"
    );
}
