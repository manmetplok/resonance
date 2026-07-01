//! Structural render tests for the External-Instrument **status states**
//! (ba todo #459, design doc #169, epic #39).
//!
//! The inspector renders four derived lifecycle states for an
//! external-instrument track:
//!
//! * **Unconfigured** — a dashed onboarding card with four numbered setup
//!   steps and an "Unconfigured" badge.
//! * **Configuring** — a warm "Configuring" badge; no onboarding, no alert.
//! * **Live** — a good "Live" badge once the MIDI out + audio return are
//!   paired and monitoring is on.
//! * **Offline** — a "Offline" badge plus an inline BAD-pink alert
//!   ("MIDI output unavailable") with *Re-scan devices* / *Pick another
//!   device…* recovery actions; the route is preserved.
//!
//! Status is derived from the config + live device flags (never stored), so
//! these tests drive the public `update()` reducer / engine-event path to
//! reach each state and assert the rendered widget tree via
//! `iced_test::Simulator::find`. Pixel goldens are blessed separately by the
//! e2e tester; these structural proofs are env-independent.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{ExternalInstrumentMessage as Eim, Message, UiMessage};
use resonance_app::state::{ExternalInstrumentStatus, TrackState, ViewMode};
use resonance_app::{theme, Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioEvent, TrackId};

const TRACK: TrackId = 1;
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

/// Fresh app on the Mixer tab with a single instrument track selected.
fn app_with_track() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(TRACK))));
    app
}

fn dispatch(app: &mut Resonance, m: Eim) {
    let _ = app.update(Message::ExternalInstrument(m));
}

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

/// Mark the track external but pick nothing — the Unconfigured state.
fn app_unconfigured() -> Resonance {
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    app
}

/// MIDI out + audio return chosen but not yet monitoring — Configuring.
fn app_configuring() -> Resonance {
    let mut app = app_unconfigured();
    dispatch(&mut app, Eim::SetMidiOutDevice(TRACK, Some("Moog Muse".into())));
    dispatch(&mut app, Eim::SetMidiOutChannel(TRACK, Some(0)));
    dispatch(&mut app, Eim::SetReturnDevice(TRACK, Some("Scarlett 18i20".into())));
    dispatch(&mut app, Eim::SetBank(TRACK, Some(31)));
    dispatch(&mut app, Eim::SetProgram(TRACK, Some(12)));
    app
}

/// Fully paired and monitoring — Live.
fn app_live() -> Resonance {
    let mut app = app_configuring();
    dispatch(&mut app, Eim::ToggleMonitor(TRACK));
    app
}

#[test]
fn unconfigured_shows_badge_and_onboarding_card() {
    let app = app_unconfigured();
    assert_eq!(
        app.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Unconfigured)
    );

    let mut ui = simulator(&app);
    ui.find("Unconfigured").expect("Unconfigured status badge");
    // The onboarding intro + all four numbered steps render.
    ui.find("External instrument track. Pair a hardware synth's MIDI output with its \
         audio return so it plays and records in-line like a built-in instrument. \
         To set it up:")
        .expect("onboarding intro line");
    ui.find("Pick the synth's MIDI output device + channel below.")
        .expect("step 1");
    ui.find("Pick the audio return input the synth is wired into.")
        .expect("step 2");
    ui.find("Choose a patch (Bank + Program) — Resonance re-sends it on load & play.")
        .expect("step 3");
    ui.find("Dial in latency compensation so the return lines up with the grid.")
        .expect("step 4");
    // No offline alert in a healthy state.
    assert!(
        ui.find("MIDI output unavailable").is_err(),
        "no offline alert while online"
    );
}

#[test]
fn configuring_shows_warm_badge_no_onboarding_no_alert() {
    let app = app_configuring();
    assert_eq!(
        app.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Configuring)
    );

    let mut ui = simulator(&app);
    ui.find("Configuring").expect("Configuring status badge");
    ui.find("EXTERNAL INSTRUMENT")
        .expect("routing group still renders");
    assert!(
        ui.find("Pick the synth's MIDI output device + channel below.")
            .is_err(),
        "onboarding card is gone once a device is picked"
    );
    assert!(
        ui.find("MIDI output unavailable").is_err(),
        "no offline alert while configuring"
    );
}

#[test]
fn live_shows_good_badge() {
    let app = app_live();
    assert_eq!(
        app.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Live)
    );

    let mut ui = simulator(&app);
    ui.find("Live").expect("Live status badge");
    assert!(
        ui.find("MIDI output unavailable").is_err(),
        "no offline alert while live"
    );
}

#[test]
fn offline_shows_bad_badge_and_recovery_alert() {
    let mut app = app_live();
    // Engine reports the configured MIDI output went offline; the route is
    // preserved and the offline flag is set (mirrored engine event).
    app.test_apply_engine_event(AudioEvent::ExternalInstrumentMidiOutOffline {
        track_id: TRACK,
        device: Some("Moog Muse".into()),
    });
    assert_eq!(
        app.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Offline)
    );

    let mut ui = simulator(&app);
    ui.find("Offline").expect("Offline status badge");
    ui.find("MIDI output unavailable")
        .expect("inline offline alert title");
    ui.find("Re-scan devices").expect("re-scan recovery action");
    ui.find("Pick another device…")
        .expect("pick-another recovery action");
    // The configured device route is preserved — the picker still carries
    // the offline device so nothing is lost.
    assert_eq!(
        app.test_external_instrument(TRACK)
            .and_then(|_| app.test_registry().tracks[0].midi_output_device.clone()),
        Some("Moog Muse".into()),
        "offline preserves the MIDI-out route"
    );
}

#[test]
fn offline_is_recoverable_via_rescan() {
    let mut app = app_live();
    app.test_apply_engine_event(AudioEvent::ExternalInstrumentMidiOutOffline {
        track_id: TRACK,
        device: Some("Moog Muse".into()),
    });
    assert_eq!(
        app.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Offline)
    );

    // "Re-scan devices" re-checks the endpoints: the offline flag is cleared
    // optimistically and, with the device back (no re-asserted offline event
    // in the headless test), the live route is restored.
    dispatch(&mut app, Eim::CheckDevices(TRACK));
    assert_eq!(
        app.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Live),
        "re-scan restores the live route"
    );
    // The assignment survived the round-trip.
    assert_eq!(
        app.test_registry().tracks[0].midi_output_device,
        Some("Moog Muse".into())
    );

    let mut ui = simulator(&app);
    ui.find("Live").expect("badge back to Live after recovery");
    assert!(
        ui.find("MIDI output unavailable").is_err(),
        "alert gone once recovered"
    );
}

/// Regression guard for the lazy-cache staleness the reviewer flagged: the
/// onboarding card and the inline "MIDI output unavailable" alert render
/// *inside* the inspector's `lazy(fp, …)` region, so `inspector_fingerprint`
/// MUST change across every transition those bodies depend on. The other
/// tests each build a fresh `Simulator` from a fresh `view()`, which bypasses
/// the retained lazy cache and so would pass even if the fingerprint were
/// stale; this one asserts on the fingerprint directly.
#[test]
fn inspector_fingerprint_changes_across_lazy_rendered_states() {
    // Unconfigured (onboarding card visible) → Configuring (card gone).
    let unconfigured = app_unconfigured();
    let fp_unconfigured = unconfigured
        .test_inspector_fingerprint(TRACK)
        .expect("fingerprint for external track");

    let configuring = app_configuring();
    let fp_configuring = configuring
        .test_inspector_fingerprint(TRACK)
        .expect("fingerprint");
    assert_ne!(
        fp_unconfigured, fp_configuring,
        "picking a device must invalidate the lazy region so the onboarding \
         card is dropped"
    );

    // Configuring → Live (monitor toggle; routing/monitor group changes).
    let live = app_live();
    let fp_live = live.test_inspector_fingerprint(TRACK).expect("fingerprint");
    assert_ne!(
        fp_configuring, fp_live,
        "enabling monitoring must invalidate the lazy region"
    );

    // Live → Offline: THE critical transition. The offline flag is set by a
    // mirrored engine event that touches no `TrackState` field, so if the
    // fingerprint didn't fold in `midi_out_offline` the retained tree would
    // never rebuild and the recovery alert would never appear.
    let mut offline = app_live();
    offline.test_apply_engine_event(AudioEvent::ExternalInstrumentMidiOutOffline {
        track_id: TRACK,
        device: Some("Moog Muse".into()),
    });
    let fp_offline = offline
        .test_inspector_fingerprint(TRACK)
        .expect("fingerprint");
    assert_ne!(
        fp_live, fp_offline,
        "going offline must invalidate the lazy region so the inline alert \
         appears (the flag is set purely in the external-instrument mirror)"
    );

    // Offline → recovered: re-scan clears the flag, so the lazy region must
    // rebuild and drop the alert from the retained tree.
    let mut recovered = offline;
    dispatch(&mut recovered, Eim::CheckDevices(TRACK));
    let fp_recovered = recovered
        .test_inspector_fingerprint(TRACK)
        .expect("fingerprint");
    assert_ne!(
        fp_offline, fp_recovered,
        "re-scan clearing the offline flag must invalidate the lazy region so \
         the alert disappears"
    );
    // …and recovery genuinely happened: the flag is cleared, status is Live
    // again, so the retained tree rebuilds without the alert. (The fingerprint
    // won't exactly equal the pre-offline Live value because CheckDevices also
    // refreshes the hardware device lists, which the fingerprint folds in.)
    assert_eq!(
        recovered.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Live),
        "re-scan restores the live route"
    );
}

#[test]
fn non_external_track_has_no_status_badge() {
    let app = app_with_track(); // never Enable-d
    assert_eq!(app.test_external_instrument_status(TRACK), None);

    let mut ui = simulator(&app);
    for label in ["Unconfigured", "Configuring", "Live", "Offline"] {
        assert!(
            ui.find(label).is_err(),
            "plain track must not render a status badge ({label})"
        );
    }
}
