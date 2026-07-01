//! Golden-image snapshots for the **media-browser panel scaffold** (design
//! doc #175, epic #35, todo #601).
//!
//! This todo builds the container only: the docked left panel in the
//! Arrange view (fixed `BROWSER_WIDTH` column, `BG_2`, `LINE` right border),
//! the "Media" chrome toggle, and the Files / Pool tab switcher. Per-tab
//! bodies land in follow-ups, so the goldens lock in the chrome, not rows.
//!
//! Three states are captured:
//!
//! 1. **hidden** — the default: no panel, the "Media" chrome toggle unlit.
//! 2. **Files tab** — panel open, Files selected (breadcrumb strip shown).
//! 3. **Pool tab** — panel open, Pool selected (breadcrumb hidden).

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{BrowserMessage, Message};
use resonance_app::state::{BrowserTab, ViewMode};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

/// Window size matches the app's default & minimum window per the
/// design guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// Build the iced simulator `Settings` with the same font registrations
/// the production app uses — without these the simulator falls back to a
/// default sans and goldens stop matching the user's reality.
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

/// Build the demo app on the Arrange tab, where the media browser lives.
fn build_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    // Belt-and-braces in case another test in this binary set STARTUP_TAB
    // to something else first (OnceLock makes our set a no-op then).
    let _ = app.update(Message::Ui(
        resonance_app::message::UiMessage::SwitchView(ViewMode::Arrange),
    ));
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

/// Baseline: browser hidden (the default), "Media" chrome toggle unlit.
#[test]
fn media_browser_hidden() {
    let app = build_app();
    snapshot_to(&app, "tests/snapshots/media_browser_hidden.png");
}

/// Panel open on the Files tab — breadcrumb strip visible.
#[test]
fn media_browser_files_tab() {
    let mut app = build_app();
    let _ = app.update(Message::Browser(BrowserMessage::ToggleVisible));
    let _ = app.update(Message::Browser(BrowserMessage::SelectTab(BrowserTab::Files)));
    snapshot_to(&app, "tests/snapshots/media_browser_files_tab.png");
}

/// Panel open on the Pool tab — breadcrumb hidden.
#[test]
fn media_browser_pool_tab() {
    let mut app = build_app();
    let _ = app.update(Message::Browser(BrowserMessage::ToggleVisible));
    let _ = app.update(Message::Browser(BrowserMessage::SelectTab(BrowserTab::Pool)));
    snapshot_to(&app, "tests/snapshots/media_browser_pool_tab.png");
}
