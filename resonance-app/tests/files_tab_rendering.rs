//! Golden-image snapshots for the **Files tab body** (design doc #175,
//! epic #35, todo #602).
//!
//! Two states are captured:
//!
//! 1. **populated** — a favourited current folder, so the breadcrumb star
//!    reads WARM; a favourites / recent pill shelf; the per-folder filter
//!    field; two subfolder rows; and four audio rows spanning the format
//!    chips (wav / flac / mp3 / ogg) with mini waveform thumbnails and
//!    durations.
//! 2. **empty folder** — an open folder with no audio: the empty-state copy
//!    plus the "Choose files…" button.
//!
//! **Snapshot files**: `tests/snapshots/files_tab_populated.png`,
//! `tests/snapshots/files_tab_empty.png`.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{BrowserMessage, Message};
use resonance_app::state::{BrowserTab, ViewMode};
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

/// Build a demo app on the Arrange tab with the browser open on the Files
/// tab. `seed` populates the Files-tab folder state.
fn build_files_app(seed: impl FnOnce(&mut Resonance)) -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Ui(
        resonance_app::message::UiMessage::SwitchView(ViewMode::Arrange),
    ));
    let _ = app.update(Message::Browser(BrowserMessage::ToggleVisible));
    let _ = app.update(Message::Browser(BrowserMessage::SelectTab(BrowserTab::Files)));
    seed(&mut app);
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

/// Files tab populated with a favourited folder, shelf, filter, subfolders,
/// and four audio rows.
#[test]
fn files_tab_populated() {
    let app = build_files_app(demo::seed_files_folder);
    snapshot_to(&app, "tests/snapshots/files_tab_populated.png");
}

/// Files tab on an empty folder — the empty-state copy + "Choose files…".
#[test]
fn files_tab_empty() {
    let app = build_files_app(demo::seed_empty_files_folder);
    snapshot_to(&app, "tests/snapshots/files_tab_empty.png");
}
