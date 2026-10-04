//! Settings → Audio I/O latency readout (W3, doc #260 finding #13):
//! `AudioCommand::QueryIoLatency` is asked for on Settings-overlay open
//! and whenever the input device list re-enumerates, and the reply is
//! mirrored onto `r.devices.io_latency` for the view to read.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::{IoLatencyInfo, ViewMode};
use resonance_app::{theme, Resonance};
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent};

use crate::common;

const WINDOW: (f32, f32) = (1440.0, 900.0);

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

#[test]
fn opening_settings_queries_io_latency() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    let rx = app.test_capture_engine();
    assert!(app.test_io_latency().is_none(), "nothing reported yet");

    let _ = app.update(Message::Ui(UiMessage::OpenSettings));
    assert!(drain(&rx).iter().any(|c| matches!(c, AudioCommand::QueryIoLatency)));
}

#[test]
fn a_device_list_change_re_queries_io_latency() {
    let (mut app, _task) = Resonance::new_for_test();
    let rx = app.test_capture_engine();

    app.test_apply_engine_event(AudioEvent::InputDevicesListed {
        devices: Vec::new(),
        default_name: None,
    });
    assert!(drain(&rx).iter().any(|c| matches!(c, AudioCommand::QueryIoLatency)));
}

#[test]
fn the_reply_is_mirrored_onto_device_state() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_apply_engine_event(AudioEvent::IoLatencyReport {
        capture_samples: 256,
        playback_samples: 128,
        round_trip_samples: 384,
    });
    let latency = app.test_io_latency().expect("mirrored");
    assert_eq!(
        latency,
        IoLatencyInfo {
            capture_samples: 256,
            playback_samples: 128,
            round_trip_samples: 384,
        }
    );
}

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

/// Settings → Audio with a populated latency reading (the default-state
/// overlay, showing the "—" placeholders, is the golden in
/// `autosave_settings_ui::settings_overlay_autosave_row`).
#[test]
fn settings_overlay_with_io_latency_golden() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    let _ = app.update(Message::Ui(UiMessage::OpenSettings));
    app.test_apply_engine_event(AudioEvent::IoLatencyReport {
        capture_samples: 256,
        playback_samples: 128,
        round_trip_samples: 384,
    });

    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/settings_overlay_io_latency.png");
}
