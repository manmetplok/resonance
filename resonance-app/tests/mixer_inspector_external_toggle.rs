//! Inspector enable/disable toggle for external-instrument mode
//! (ba todo #1065, doc #251 gap 1 affordance 1).
//!
//! Before this affordance, no view code dispatched
//! `ExternalInstrumentMessage::Enable/Disable`, so the whole
//! external-instrument UI surface (onboarding card, routing group, pickers)
//! was unreachable through the UI. These tests drive the inspector purely
//! via UI-dispatched messages:
//!  - a plain instrument track renders the "External hardware instrument"
//!    enable affordance in the ROUTING group;
//!  - pressing it lands the user on the onboarding ("Unassigned") card;
//!  - the external group offers a "Use built-in instrument" disable action
//!    that returns the plain ROUTING view;
//!  - audio / vocal tracks never grow the toggle;
//!  - the enable → disable transition round-trips through undo / redo.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{ExternalInstrumentMessage as Eim, Message, UiMessage};
use resonance_app::state::{TrackState, ViewMode};
use resonance_app::{theme, Resonance, STARTUP_TAB};
use resonance_audio::types::TrackId;

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

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

/// Fresh app on the Mixer tab with a single **plain** instrument track
/// selected (not yet external).
fn app_with_instrument_track() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    // Undo snapshots key off the project path — set one so Enable/Disable
    // are recorded and reversible (mirrors the device-preset undo test).
    app.test_set_project_path(std::path::PathBuf::from(
        "/tmp/resonance-test-1065",
    ));
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(TRACK))));
    app
}

/// A plain instrument track shows the enable affordance in ROUTING, and no
/// external-instrument surface yet.
#[test]
fn plain_instrument_shows_enable_affordance() {
    let app = app_with_instrument_track();
    let mut ui = simulator(&app);

    ui.find("ROUTING").expect("generic ROUTING group");
    ui.find("External hardware instrument")
        .expect("enable affordance in the ROUTING group");
    assert!(
        ui.find("EXTERNAL INSTRUMENT").is_err(),
        "plain track must not render the External Instrument group"
    );
}

/// Enabling via the inspector message lands the user on the onboarding
/// ("Unassigned") card and swaps ROUTING for the External Instrument group.
#[test]
fn enable_reaches_onboarding_card() {
    let mut app = app_with_instrument_track();
    let _ = app.update(Message::ExternalInstrument(Eim::Enable(TRACK)));

    let mut ui = simulator(&app);
    // A freshly-enabled track is Unconfigured — the status badge appears in
    // the title row and the dashed onboarding ("Unassigned") card renders
    // before the routing pickers. Both are unique to the onboarding state.
    ui.find("Unconfigured")
        .expect("Unconfigured status badge after Enable");
    ui.find("Pick the synth's MIDI output device + channel below.")
        .expect("onboarding card step 1 after Enable");
    ui.find("EXTERNAL INSTRUMENT")
        .expect("External Instrument group replaces generic ROUTING");
    // The enable affordance is gone once the track is external.
    assert!(
        ui.find("External hardware instrument").is_err(),
        "enable affordance must not persist on an external track"
    );
}

/// The external group offers the disable action, and dispatching it returns
/// the plain instrument ROUTING view (external surface removed).
#[test]
fn disable_returns_plain_routing() {
    let mut app = app_with_instrument_track();
    let _ = app.update(Message::ExternalInstrument(Eim::Enable(TRACK)));

    // The disable affordance is present on the external track.
    {
        let mut ui = simulator(&app);
        ui.find("Use built-in instrument")
            .expect("disable affordance in the External Instrument group");
    }

    let _ = app.update(Message::ExternalInstrument(Eim::Disable(TRACK)));

    let mut ui = simulator(&app);
    ui.find("ROUTING").expect("generic ROUTING group restored");
    ui.find("External hardware instrument")
        .expect("enable affordance back after disable");
    assert!(
        ui.find("EXTERNAL INSTRUMENT").is_err(),
        "External Instrument group removed after Disable"
    );
}

/// Enable → Disable round-trips through undo / redo: undo restores the
/// external group + onboarding card, redo removes it again. The undo
/// classifier records Enable/Disable via `UndoExtras.external_instruments`.
#[test]
fn enable_disable_undo_redo_round_trip() {
    let mut app = app_with_instrument_track();
    let _ = app.update(Message::ExternalInstrument(Eim::Enable(TRACK)));
    assert!(
        app.test_external_instrument(TRACK).is_some(),
        "Enable makes the track external"
    );
    let _ = app.update(Message::ExternalInstrument(Eim::Disable(TRACK)));

    // After disable the track is no longer external and the plain ROUTING
    // view is shown.
    assert!(
        app.test_external_instrument(TRACK).is_none(),
        "Disable removes the external entry"
    );
    {
        let mut ui = simulator(&app);
        ui.find("ROUTING").expect("plain routing after disable");
        assert!(ui.find("EXTERNAL INSTRUMENT").is_err());
    }

    // Undo the disable → the external group (and onboarding card) return.
    let _ = app.update(Message::Undo);
    assert!(
        app.test_external_instrument(TRACK).is_some(),
        "undo restores the external entry"
    );
    {
        let mut ui = simulator(&app);
        ui.find("EXTERNAL INSTRUMENT")
            .expect("undo restores the External Instrument group");
        ui.find("Unconfigured")
            .expect("undo restores the onboarding status badge");
    }

    // Redo the disable → back to the plain instrument routing view.
    let _ = app.update(Message::Redo);
    assert!(
        app.test_external_instrument(TRACK).is_none(),
        "redo re-applies the disable"
    );
    {
        let mut ui = simulator(&app);
        ui.find("ROUTING").expect("redo re-applies the disable");
        assert!(
            ui.find("EXTERNAL INSTRUMENT").is_err(),
            "redo removes the External Instrument group again"
        );
    }
}

/// Audio and vocal tracks must never grow the enable toggle (mirrors the
/// EXTERNAL INSTRUMENT group's instrument-only gating).
#[test]
fn audio_and_vocal_tracks_have_no_toggle() {
    for track in [TrackState::new_audio(TRACK, 0), TrackState::new_vocal(TRACK, 0)] {
        let _ = STARTUP_TAB.set(ViewMode::Mixer);
        let (mut app, _task) = Resonance::new();
        app.test_set_active_project(true);
        app.test_push_track(track);
        let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
        let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(TRACK))));

        let mut ui = simulator(&app);
        assert!(
            ui.find("External hardware instrument").is_err(),
            "non-instrument tracks must not render the enable toggle"
        );
    }
}
