//! Golden-image snapshot for the **remote-control-active indicator** in
//! the window chrome (ba doc #265, epic #200, todo #1159 — the only UI in
//! the epic).
//!
//! Captured state: one control client connected, so the chrome shows the
//! small green "Remote" chip just right of the project-title cluster. The
//! hidden (zero-client) state is asserted separately below by rendering
//! and confirming the view builds — there is no golden for it because it
//! is byte-for-byte the normal chrome (the indicator collapses to a
//! zero-width Space, no layout shift).
//!
//! **Snapshot file**: `tests/snapshots/remote_indicator_active.png`.

mod common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::control_socket::ControlMessage;
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::ViewMode;
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

/// A demo app on the Arrange tab with `connected` control clients wired
/// in through the normal `Connected` events (mirrors the socket bridge).
fn build_app(connected: u64) -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Arrange)));
    for conn in 0..connected {
        let _ = app.update(Message::Control(ControlMessage::Connected { conn }));
    }
    app
}

/// One control client connected — the chrome shows the "Remote" chip.
#[test]
fn remote_indicator_active() {
    let app = build_app(1);
    assert_eq!(app.control_client_count(), 1);

    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/remote_indicator_active.png");
}

/// No client connected — the indicator is hidden and the view still
/// renders (no panic, no layout dependency on the chip).
#[test]
fn remote_indicator_hidden_when_no_clients() {
    let app = build_app(0);
    assert_eq!(app.control_client_count(), 0);

    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    ui.snapshot(&theme::resonance_theme())
        .expect("chrome renders with the indicator hidden");
}
