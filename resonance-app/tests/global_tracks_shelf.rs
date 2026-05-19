//! Golden-image snapshots for the **Arrange-view global-tracks shelf**
//! — the collapsible strip that sits between the section-pill band
//! and the regular track lanes, holding the chord / tempo / signature
//! lanes plus the always-visible "GLOBAL · 6/8 · 90 BPM · …" summary
//! line.
//!
//! Two states are locked in here:
//!
//! 1. **collapsed** — only the 32 px shelf header strip is visible
//!    (caret + `GLOBAL` tag + count badge on the column side,
//!    summary text on the canvas side). Below the strip the regular
//!    tracks start immediately.
//! 2. **expanded** — the shelf header strip plus three lane rows
//!    (chord lane with section tabs + chord blocks, tempo automation
//!    curve, signature pill). Track lanes start at the very bottom
//!    of the shelf.
//!
//! Both snapshots are taken at scroll = 0 so the alignment between
//! the column-side labels (chord / tempo / signature) and the canvas
//! lanes is locked in. The companion
//! `track_header_alignment_scroll_*` suite covers the fractional
//! vertical-scroll variants; this file focuses on the shelf chrome
//! itself.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

/// Window size matches the app's default & minimum window per the
/// design guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// Build the iced simulator `Settings` with the same font registrations
/// the production app uses — without these the simulator falls back to
/// a default sans and the goldens stop matching the user's reality.
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

/// Build a fully-seeded demo app on the Arrange tab. The
/// `expand_shelf` flag toggles the global-tracks shelf to its
/// expanded state.
fn build_app(expand_shelf: bool) -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    if expand_shelf {
        let _ = app.update(Message::Ui(UiMessage::ToggleGlobalTracks));
    }
    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(
        sim_settings(),
        Size::new(WINDOW.0, WINDOW.1),
        app.view(),
    );
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

/// Collapsed state — only the 32 px summary strip is visible.
#[test]
fn global_tracks_shelf_collapsed() {
    let app = build_app(false);
    snapshot_to(&app, "tests/snapshots/global_tracks_shelf_collapsed.png");
}

/// Expanded state — shelf header + chord / tempo / signature lanes.
#[test]
fn global_tracks_shelf_expanded() {
    let app = build_app(true);
    snapshot_to(&app, "tests/snapshots/global_tracks_shelf_expanded.png");
}
