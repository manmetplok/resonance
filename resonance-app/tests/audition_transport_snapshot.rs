//! Golden-image snapshots for the **audition transport bar** pinned to the
//! bottom of the media-browser panel (design doc #175, epic #35, todo #604).
//!
//! Two states are captured:
//!
//! 1. **idle** — a row selected-to-audition but no preview sounding: the
//!    play button, the selected row's scrub waveform (playhead at the start),
//!    a `0:00 / M:SS` readout, and the neutral Auto-play / Loop / Sync chips.
//! 2. **playing** — a preview sounding ~40 % through with Auto-play + Loop +
//!    Sync on: the stop button, the WARM played-span + mid-strip playhead,
//!    the advanced readout, the WARM-lit toggle chips, and the WARM
//!    playing-row highlight in the listing above.
//!
//! **Snapshot files**: `tests/snapshots/audition_transport_idle.png`,
//! `tests/snapshots/audition_transport_playing.png`.

mod common;

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
/// tab. `seed` populates the folder + audition transport state.
fn build_app(seed: impl FnOnce(&mut Resonance)) -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new_for_test();
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
    common::assert_golden(&snap, path);
}

/// Idle transport — a selected row, no preview sounding, toggles off.
#[test]
fn audition_transport_idle() {
    let app = build_app(demo::seed_audition_idle);
    snapshot_to(&app, "tests/snapshots/audition_transport_idle.png");
}

/// Playing transport — a preview ~40 % through with Loop + Sync on.
#[test]
fn audition_transport_playing() {
    let app = build_app(demo::seed_audition_playing);
    snapshot_to(&app, "tests/snapshots/audition_transport_playing.png");
}
