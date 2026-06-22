//! Golden-image snapshot for arrangement markers in the timeline ruler
//! (todo #368 / doc #161).
//!
//! Locks in the ruler-band rendering added by todo #368: a point marker
//! draws as a colour-tinted flag (pole + pennant + name label) and a
//! ranged marker draws as a translucent labelled span with start/end
//! edge lines, both distinct from the amber loop range and the Compose
//! section pills.
//!
//! Window size matches the app's default 1440×900 per
//! `ux-guidelines.md`. On first run `matches_image()` writes the golden
//! under `tests/snapshots/`; subsequent runs diff against the committed
//! PNG.

use iced::Size;
use iced_test::simulator::Simulator;
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

/// Demo app pinned to the Arrange tab so the timeline canvas (and its
/// ruler band) is on screen, seeded with a point marker and a ranged
/// region marker at deterministic sample positions near the start of the
/// arrangement so both fall inside the default viewport.
fn build_arrange_app_with_markers() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);

    // Point marker — a red flag a little way into the arrangement.
    app.test_add_marker(ArrangementMarker::new_point(
        1,
        "Verse".to_string(),
        [0xE5, 0x4B, 0x4B],
        96_000,
    ));
    // Ranged region marker — a blue translucent span further along.
    app.test_add_marker(ArrangementMarker::new_region(
        2,
        "Chorus".to_string(),
        [0x3D, 0x8B, 0xE5],
        240_000,
        432_000,
    ));

    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

#[test]
fn timeline_markers_render_in_ruler() {
    let app = build_arrange_app_with_markers();
    snapshot_to(&app, "tests/snapshots/timeline_markers_render_in_ruler.png");
}
