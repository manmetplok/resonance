//! Golden-image snapshot for the **Pool tab body** (design doc #175,
//! epic #35, todo #603).
//!
//! Renders the Pool tab with three representative pool assets:
//!
//! 1. **Used** — a stereo WAV, 4.5 s, referenced by one audio clip.
//!    Renders `used ×1` badge in lavender.
//! 2. **Unused** — a mono FLAC, 2.0 s, no clips reference it.
//!    Renders `unused` badge in muted text.
//! 3. **Missing** — a WAV whose backing file is absent (`missing: true`).
//!    Renders a BAD-coloured ⚠ glyph and an inline `relink` chip.
//!
//! The Pool tab must hide the filesystem breadcrumb (project-level list,
//! not a path); this is verified by asserting the panel shows the tab
//! switcher without a breadcrumb row preceding the asset list.
//!
//! **Snapshot file**: `tests/snapshots/pool_tab_populated.png`

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{BrowserMessage, Message};
use resonance_app::state::{BrowserTab, ViewMode};
use resonance_app::{demo, theme, Resonance};

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

/// Build a populated app on the Arrange tab with the browser open on Pool
/// and three demo assets seeded (used / unused / missing).
fn build_pool_app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    demo::seed_demo_content(&mut app);
    // Seed pool assets: used (×1) / unused / missing
    demo::seed_pool_assets(&mut app);
    // Switch to Arrange through the real reducer, so the browser opens
    // over the arrangement the way it does in the app.
    let _ = app.update(Message::Ui(
        resonance_app::message::UiMessage::SwitchView(ViewMode::Arrange),
    ));
    // Open the browser on the Pool tab
    let _ = app.update(Message::Browser(BrowserMessage::ToggleVisible));
    let _ = app.update(Message::Browser(BrowserMessage::SelectTab(BrowserTab::Pool)));
    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// Pool tab populated with all three asset variants (used / unused / missing).
#[test]
fn pool_tab_populated() {
    let app = build_pool_app();
    snapshot_to(&app, "tests/snapshots/pool_tab_populated.png");
}
