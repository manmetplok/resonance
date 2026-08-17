//! View-half coverage for the Mixer-inspector external-instrument
//! "Auto-detect (ping)" latency button (ba todo #1069, view half of Gap 2,
//! doc #251; state half is #1068, engine ping is #453).
//!
//! Drives the public `update()` reducer + `test_apply_engine_event` /
//! `test_set_transport_playing` to reach each button state, then renders the
//! inspector via `iced_test::Simulator` and asserts:
//!   * idle + stopped transport: clicking the button emits
//!     `ExternalInstrumentMessage::DetectLatency(track)`,
//!   * while measuring: the label reads "Measuring…" and the control is
//!     non-interactive (no message on click),
//!   * a failed detect renders its reason as a caption under the button.
//!
//! These are structural (widget-tree) proofs, independent of the pixel
//! goldens the e2e tester blesses separately.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{ExternalInstrumentMessage as Eim, Message, UiMessage};
use resonance_app::state::{TrackState, ViewMode};
use resonance_app::{theme, Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioEvent, TrackId};

const TRACK: TrackId = 1;
// Tall enough that the whole external-instrument inspector column — down to
// the latency block near the bottom — is laid out within the viewport, so the
// ping button has real (clickable) `visible_bounds` in the simulator.
const WINDOW: (f32, f32) = (1440.0, 1800.0);

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

/// Fresh app on the Mixer tab with a single instrument track marked as an
/// external instrument and selected, so `view()` renders the External
/// Instrument inspector group (mirrors `mixer_inspector_external_instrument`).
fn app_with_external_track() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(TRACK))));
    dispatch(&mut app, Eim::Enable(TRACK));
    app
}

fn dispatch(app: &mut Resonance, m: Eim) {
    let _ = app.update(Message::ExternalInstrument(m));
}

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

/// Collect the messages a single click on `label` produces (empty when the
/// click doesn't land on an interactive control).
fn click_messages(app: &Resonance, label: &str) -> Vec<Message> {
    let mut ui = simulator(app);
    ui.click(label)
        .unwrap_or_else(|e| panic!("clicking {label:?} should hit a control: {e:?}"));
    ui.into_messages().collect()
}

/// Idle + transport stopped: the button is interactive and clicking it emits
/// `DetectLatency` for the selected track.
#[test]
fn idle_button_emits_detect_latency() {
    let app = app_with_external_track();

    // The button is labelled "Auto-detect (ping)" when idle.
    {
        let mut ui = simulator(&app);
        ui.find("Auto-detect (ping)")
            .expect("idle ping button label");
    }

    let msgs = click_messages(&app, "Auto-detect (ping)");
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            Message::ExternalInstrument(Eim::DetectLatency(TRACK))
        )),
        "idle button click emits DetectLatency, got {msgs:?}"
    );
}

/// While a ping is in flight the label reads "Measuring…" and the control is
/// disabled — clicking it emits nothing, so a double-press is impossible.
#[test]
fn measuring_button_is_non_interactive() {
    let mut app = app_with_external_track();
    // Kick off a detect so the state flips to in-progress.
    dispatch(&mut app, Eim::DetectLatency(TRACK));
    assert!(
        app.test_external_instrument(TRACK)
            .unwrap()
            .latency_detect_in_progress,
        "detect is in flight after the press"
    );

    {
        let mut ui = simulator(&app);
        ui.find("Measuring…")
            .expect("measuring label while detect is in flight");
        assert!(
            ui.find("Auto-detect (ping)").is_err(),
            "idle label is replaced while measuring"
        );
    }

    // Clicking the measuring button must not re-dispatch DetectLatency.
    let msgs = click_messages(&app, "Measuring…");
    assert!(
        !msgs
            .iter()
            .any(|m| matches!(m, Message::ExternalInstrument(Eim::DetectLatency(_)))),
        "measuring button is disabled — no DetectLatency, got {msgs:?}"
    );
}

/// A failed auto-detect renders its reason as a caption under the button.
#[test]
fn failure_reason_caption_renders() {
    let mut app = app_with_external_track();
    // A clean failure stores the reason and clears the in-flight guard.
    app.test_apply_engine_event(AudioEvent::ExternalInstrumentLatencyDetectFailed {
        track_id: TRACK,
        reason: "No return detected within the listen window.".into(),
    });
    assert_eq!(
        app.test_external_instrument(TRACK)
            .unwrap()
            .latency_detect_error
            .as_deref(),
        Some("No return detected within the listen window."),
        "state stores the failure reason for the view to surface"
    );

    let mut ui = simulator(&app);
    ui.find("No return detected within the listen window.")
        .expect("failure reason rendered as a caption under the button");
    // Idle again after the failure resolved — the button is pressable once more.
    ui.find("Auto-detect (ping)")
        .expect("button returns to its idle label after a failure");
}

/// Playing transport: the idle label stays but the button has no `on_press`,
/// so clicking it emits nothing (the engine ping requires a stopped transport).
#[test]
fn playing_transport_button_is_non_interactive() {
    let mut app = app_with_external_track();
    app.test_set_transport_playing(true);

    let msgs = click_messages(&app, "Auto-detect (ping)");
    assert!(
        !msgs
            .iter()
            .any(|m| matches!(m, Message::ExternalInstrument(Eim::DetectLatency(_)))),
        "button is inert while the transport is playing, got {msgs:?}"
    );
}
