//! Structural render tests for the Mixer-inspector **External Instrument**
//! ROUTING group (ba todo #457, design doc #169, epic #39).
//!
//! An external-instrument track has no track-type discriminant — it's
//! marked purely by presence in the `external_instruments` map (#454).
//! When marked, the inspector replaces the generic ROUTING fields with
//! the dedicated External Instrument group (MIDI Output, Audio Return,
//! Patch, Latency Compensation, Return Monitoring, Output) and retitles
//! SIGNAL → "Signal · Return" because the metered signal is the hardware
//! return.
//!
//! These drive the public `update()` reducer to configure a track, then
//! assert the rendered widget tree via `iced_test::Simulator::find`.
//! Pixel goldens for this group are blessed separately by the e2e
//! tester; these structural proofs are env-independent.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{ExternalInstrumentMessage as Eim, Message, UiMessage};
use resonance_app::state::{TrackState, ViewMode};
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

/// Fresh app on the Mixer tab with a single instrument track marked as
/// an external instrument and selected, so `view()` renders the External
/// Instrument inspector group.
fn app_with_external_track() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    // Belt-and-braces in case another test in this binary set the tab
    // first (OnceLock makes our `set` above a no-op then).
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

/// A fully-configured external-instrument track renders every section of
/// the External Instrument group, and the SIGNAL group is retitled.
#[test]
fn external_instrument_group_renders_all_sections() {
    let mut app = app_with_external_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    dispatch(&mut app, Eim::SetMidiOutDevice(TRACK, Some("Moog Muse".into())));
    dispatch(&mut app, Eim::SetMidiOutChannel(TRACK, Some(0)));
    dispatch(&mut app, Eim::SetReturnDevice(TRACK, Some("Scarlett 18i20".into())));
    dispatch(&mut app, Eim::SetBank(TRACK, Some(31)));
    dispatch(&mut app, Eim::SetProgram(TRACK, Some(12)));
    dispatch(&mut app, Eim::SetLatencyOffset(TRACK, 512));
    dispatch(&mut app, Eim::ToggleMonitor(TRACK));

    let mut ui = simulator(&app);

    // The ROUTING group becomes the External Instrument group, and the
    // SIGNAL group reads "Signal · Return".
    ui.find("EXTERNAL INSTRUMENT")
        .expect("External Instrument group header");
    ui.find("SIGNAL · RETURN")
        .expect("SIGNAL group retitled to Signal · Return");

    // All six sub-sections of the group are present.
    ui.find("MIDI OUTPUT").expect("MIDI Output field");
    ui.find("AUDIO RETURN").expect("Audio Return field");
    ui.find("PATCH").expect("Patch field");
    ui.find("LATENCY COMPENSATION").expect("Latency field");
    ui.find("RETURN MONITORING").expect("Return Monitoring field");

    // Patch tiles read the configured bank/program (zero-padded).
    ui.find("031").expect("bank tile shows 031");
    ui.find("012").expect("program tile shows 012");

    // The monitor / arm toggles and the epic-#40 preset affordance.
    ui.find("Input monitor").expect("input monitor toggle");
    ui.find("Record arm").expect("record arm toggle");
    ui.find("Muse preset →").expect("disabled preset affordance");
}

/// A non-external track keeps the generic ROUTING group — the External
/// Instrument header must not appear and SIGNAL keeps its plain title.
#[test]
fn non_external_track_keeps_generic_routing() {
    let app = app_with_external_track();
    // No `Enable` — the track is a plain instrument track.

    let mut ui = simulator(&app);
    ui.find("ROUTING").expect("generic ROUTING group");
    assert!(
        ui.find("EXTERNAL INSTRUMENT").is_err(),
        "plain track must not render the External Instrument group"
    );
    assert!(
        ui.find("SIGNAL · RETURN").is_err(),
        "plain track keeps the plain SIGNAL title"
    );
}

/// An offline MIDI output keeps the device route (stale-override) and
/// surfaces the BAD-pink "offline" tag beside the MIDI Output label.
#[test]
fn offline_midi_output_shows_offline_tag() {
    let mut app = app_with_external_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    dispatch(&mut app, Eim::SetMidiOutDevice(TRACK, Some("Moog Muse".into())));
    // Engine reports the configured MIDI output went offline; the route
    // is preserved and the offline flag is set (mirrored by #455's path).
    app.test_apply_engine_event(AudioEvent::ExternalInstrumentMidiOutOffline {
        track_id: TRACK,
        device: Some("Moog Muse".into()),
    });

    let mut ui = simulator(&app);
    ui.find("EXTERNAL INSTRUMENT")
        .expect("External Instrument group renders while offline");
    ui.find("offline").expect("BAD-pink offline tag on MIDI Output");
}
