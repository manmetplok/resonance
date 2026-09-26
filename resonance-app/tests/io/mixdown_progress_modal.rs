//! WAV mixdown progress modal (code review FU-F1c).
//!
//! A WAV mixdown (`io.bouncing`) gates GUI traffic exactly like a bounce
//! in place, but used to show nothing except a tiny "Bouncing..." label on
//! the master strip — the app just looked frozen. It now shows the shared
//! bounce / freeze progress overlay, fed by the engine's whole-percent
//! `BounceProgress` events, with a Cancel that flips the export's
//! cooperative cancel token (`AudioCommand::CancelBounce`). The engine
//! answers a cancel with `BounceError("Bounce cancelled")`, which clears
//! the modal without an error banner.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, ProjectIoMessage, TransportMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent};

const WINDOW: (f32, f32) = (1440.0, 900.0);
const MIXDOWN: &str = "/tmp/io-mixdown-progress-modal/My Song.wav";

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

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

fn start_mixdown(app: &mut Resonance) {
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::BouncePathSelected(Some(
        MIXDOWN.to_owned(),
    ))));
    assert!(app.test_is_bouncing(), "sanity: the mixdown is in flight");
}

/// Demo content on Arrange, mid-mixdown, with the engine command capture.
fn app_mid_mixdown() -> (
    Resonance,
    resonance_audio::test_support::Receiver<AudioCommand>,
) {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    demo::seed_demo_content(&mut app);
    app.test_set_active_project(true);
    start_mixdown(&mut app);
    while cmd_rx.try_recv().is_ok() {}
    (app, cmd_rx)
}

#[test]
fn modal_shows_the_target_and_live_progress_while_the_mixdown_renders() {
    let (mut app, _cmd_rx) = app_mid_mixdown();
    {
        let mut ui = simulator(&app);
        ui.find("Bouncing \"My Song.wav\"")
            .expect("the mixdown modal must name the target file");
        ui.find("0%").expect("the modal starts at 0%");
        ui.find("Cancel").expect("the mixdown export can be cancelled");
    }

    app.test_apply_engine_event(AudioEvent::BounceProgress { fraction: 0.42 });
    let mut ui = simulator(&app);
    ui.find("42%")
        .expect("BounceProgress must drive the mixdown modal's caption");
}

#[test]
fn modal_clears_when_the_mixdown_completes() {
    let (mut app, _cmd_rx) = app_mid_mixdown();
    app.test_apply_engine_event(AudioEvent::BounceComplete {
        path: MIXDOWN.to_owned(),
    });
    let mut ui = simulator(&app);
    assert!(
        ui.find("Bouncing \"My Song.wav\"").is_err(),
        "no mixdown modal once the export is done"
    );
}

#[test]
fn modal_absent_when_idle() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    demo::seed_demo_content(&mut app);
    let mut ui = simulator(&app);
    assert!(ui.find("Cancel").is_err(), "no progress modal while idle");
}

#[test]
fn cancel_passes_the_gate_stops_the_export_and_raises_no_banner() {
    let (mut app, cmd_rx) = app_mid_mixdown();

    // Everything else stays gated mid-render.
    let _ = app.update(Message::Transport(TransportMessage::Play));
    assert!(!cmd_rx.try_iter().any(|c| matches!(c, AudioCommand::Play)));

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::CancelBounce));
    let sent: Vec<AudioCommand> = cmd_rx.try_iter().collect();
    assert_eq!(
        sent.iter().filter(|c| matches!(c, AudioCommand::CancelBounce)).count(),
        1,
        "Cancel must reach the engine exactly once, got {sent:?}"
    );
    // A second press while the cancel is pending is a no-op.
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::CancelBounce));
    assert!(!cmd_rx.try_iter().any(|c| matches!(c, AudioCommand::CancelBounce)));
    {
        let mut ui = simulator(&app);
        ui.find("0% \u{00b7} cancelling")
            .expect("the modal must acknowledge the pending cancel");
    }

    // The engine confirms the cancel the way `cancel_cleanup` does.
    app.test_apply_engine_event(AudioEvent::BounceError("Bounce cancelled".into()));
    assert!(!app.test_is_bouncing(), "the cancelled mixdown must clear");
    assert!(
        !app.test_error_message_is_set(),
        "a user cancel is not an error: {:?}",
        app.test_error_message()
    );
}

#[test]
fn a_real_failure_after_an_earlier_cancel_still_raises_the_banner() {
    let (mut app, _cmd_rx) = app_mid_mixdown();
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::CancelBounce));
    app.test_apply_engine_event(AudioEvent::BounceError("Bounce cancelled".into()));

    start_mixdown(&mut app);
    app.test_apply_engine_event(AudioEvent::BounceError("disk full".into()));
    assert_eq!(app.test_error_message(), Some("Bounce failed: disk full"));
}

#[test]
fn mixdown_modal_golden() {
    let (mut app, _cmd_rx) = app_mid_mixdown();
    app.test_apply_engine_event(AudioEvent::BounceProgress { fraction: 0.58 });
    let mut ui = simulator(&app);
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/mixdown_progress_modal.png");
}
