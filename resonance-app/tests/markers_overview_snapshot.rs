//! Golden-image snapshot for the arrangement-markers overview popover
//! (todo #370 / doc #161).
//!
//! Locks in the overview panel added by todo #370: the transport flag
//! button opens a popover anchored under the transport bar listing each
//! marker as a colour swatch + name (region markers prefixed with a "◇"
//! glyph) + bar position; clicking a row seeks the playhead. This snapshot
//! renders the app with the overview open over a demo arrangement.
//!
//! Window size matches the app's default 1440×900 per `ux-guidelines.md`.
//! On first run `matches_image()` writes the golden under
//! `tests/snapshots/`; subsequent runs diff against the committed PNG.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::{ArrangementMarker, ViewMode};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

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

/// Demo app on the Arrange tab, seeded with a mix of point and ranged
/// markers, with the markers overview popover toggled open.
fn build_app_with_overview_open() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);

    app.test_add_marker(ArrangementMarker::new_point(
        1,
        "Intro".to_string(),
        [0xE5, 0x9B, 0x33],
        48_000,
    ));
    app.test_add_marker(ArrangementMarker::new_point(
        2,
        "Verse".to_string(),
        [0xE5, 0x4B, 0x4B],
        96_000,
    ));
    app.test_add_marker(ArrangementMarker::new_region(
        3,
        "Chorus".to_string(),
        [0x3D, 0x8B, 0xE5],
        240_000,
        432_000,
    ));

    // Open the overview popover (the transport flag button).
    let _ = app.update(Message::Ui(UiMessage::ToggleMarkersOverview));
    app
}

#[test]
fn markers_overview_popover_renders() {
    let app = build_app_with_overview_open();
    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image("tests/snapshots/markers_overview_popover_renders.png")
            .expect("matches_image i/o"),
        "snapshot diverged from golden"
    );
}
