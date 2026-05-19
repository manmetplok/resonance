//! Golden-image snapshots for the Arrange view's track-header column.
//!
//! The track-header column has to mirror the timeline canvas's vertical
//! layout row-for-row so each header stays glued to its lane during
//! vertical scrolling. Three regressions are locked in here:
//!
//! 1. **scroll = 0** — the section-band placeholder pushes the first
//!    header down to align with its lane (without it every header
//!    drifts up by `SECTION_BAND_HEIGHT`).
//! 2. **scroll = 50** — fractional scroll inside the first row. This is
//!    the case that exposed the snap-to-row bug where the column would
//!    only translate in multiples of `TRACK_HEIGHT`.
//! 3. **scroll = 140** — past row 1 plus a 44 px fractional offset, so
//!    multi-row skipping plus fractional translation both have to
//!    cooperate.
//!
//! Window size is the app's 1440×900 minimum (per `ux-guidelines.md`).
//! On first run `matches_image()` writes the goldens under
//! `tests/snapshots/`; subsequent runs diff against the committed PNGs.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, ViewportMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

/// Window size matches the app's default & minimum window per the
/// design guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// Build the iced simulator `Settings` so the headless renderer sees
/// the same fonts the production app registers in `main.rs`. Without
/// these, the simulator falls back to a default sans and the goldens
/// stop matching what the user actually sees.
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

/// Build a fully-seeded demo app at the requested vertical scroll
/// offset. Uses the public `ViewportMessage::ScrollToY` path so the
/// real reducer + clamping logic runs — the test doesn't poke
/// `viewport.scroll_offset_y` directly.
fn build_app_scrolled(scroll_y: f32) -> Resonance {
    // STARTUP_TAB is a process-global OnceLock — set it once to Arrange
    // so the first test to construct an app pins the startup view.
    // Subsequent `.set` calls are no-ops, so other tests in this file
    // share the same value (which is what we want).
    let _ = STARTUP_TAB.set(ViewMode::Arrange);

    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);

    // Inform the reducer of the on-screen viewport so `ScrollToY`'s
    // clamping uses realistic bounds. Without this, the content-height
    // clamp can pin the offset to zero on the first scroll.
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportWidth(
        WINDOW.0 - theme::TRACK_HEADER_WIDTH,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0,
        // Plenty of vertical headroom so the clamp can't pin us back.
        WINDOW.1 * 4.0,
    )));

    if scroll_y > 0.0 {
        let _ = app.update(Message::Viewport(ViewportMessage::ScrollToY(scroll_y)));
    }
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

#[test]
fn track_header_alignment_scroll_0() {
    let app = build_app_scrolled(0.0);
    snapshot_to(&app, "tests/snapshots/track_header_alignment_scroll_0.png");
}

#[test]
fn track_header_alignment_scroll_50() {
    let app = build_app_scrolled(50.0);
    snapshot_to(
        &app,
        "tests/snapshots/track_header_alignment_scroll_50.png",
    );
}

#[test]
fn track_header_alignment_scroll_140() {
    let app = build_app_scrolled(140.0);
    snapshot_to(
        &app,
        "tests/snapshots/track_header_alignment_scroll_140.png",
    );
}
