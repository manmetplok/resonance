//! Structural render tests for the Mixer **track strip** of an
//! external-instrument track (ba todo #458, design doc #169, epic #39).
//!
//! An external-instrument track is marked purely by presence in the
//! `external_instruments` map (#454) — no track-type discriminant. When
//! marked, its channel strip grows a lavender `Ext` pill in the head plus
//! three summary chips under it: **MIDI** (device · channel + activity
//! dot), **Return** (input device · channels) and **Patch** (bank/program).
//! The Mon / record-arm buttons and the fader meters are unchanged — they
//! already read the same `TrackState` the inspector toggles, so the two
//! surfaces share one source of truth.
//!
//! These drive the public `update()` reducer to configure a track, then
//! assert the rendered strip widget tree via `iced_test::Simulator::find`.
//! Pixel goldens are blessed separately by the e2e tester; these
//! structural proofs are env-independent.

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

/// Fresh app on the Mixer tab with a single instrument track. The track is
/// *not* selected, so only the channel strip renders (no inspector) and
/// the assertions isolate strip-specific widgets.
fn app_with_track() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    app
}

fn dispatch(app: &mut Resonance, m: Eim) {
    let _ = app.update(Message::ExternalInstrument(m));
}

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

/// A configured external-instrument strip shows the `Ext` pill and the
/// three summary chips with their device/channel/patch values.
#[test]
fn external_strip_shows_pill_and_chips() {
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    dispatch(&mut app, Eim::SetMidiOutDevice(TRACK, Some("Muse".into())));
    dispatch(&mut app, Eim::SetMidiOutChannel(TRACK, Some(0)));
    dispatch(&mut app, Eim::SetReturnDevice(TRACK, Some("Scarlett".into())));
    dispatch(&mut app, Eim::SetBank(TRACK, Some(31)));
    dispatch(&mut app, Eim::SetProgram(TRACK, Some(12)));

    let mut ui = simulator(&app);

    // The lavender `Ext` pill replaces the (absent) Inst/Audio tag.
    ui.find("Ext").expect("Ext pill in the strip head");

    // The three chip keys.
    ui.find("MIDI").expect("MIDI chip key");
    ui.find("Return").expect("Return chip key");
    ui.find("Patch").expect("Patch chip key");

    // Chip values mirror the configured MIDI out, audio return and patch.
    ui.find("Muse \u{b7} Ch 1").expect("MIDI chip device + channel");
    ui.find("Scarlett In 1/2").expect("Return chip device + port");
    ui.find("Bank 31 \u{b7} Prog 12").expect("Patch chip bank + program");
}

/// An external strip with nothing paired yet still shows the pill + chips,
/// with placeholder values (so the strip reads as "external, unconfigured"
/// rather than a plain track).
#[test]
fn unconfigured_external_strip_shows_placeholders() {
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));

    let mut ui = simulator(&app);
    ui.find("Ext").expect("Ext pill even when unconfigured");
    ui.find("MIDI").expect("MIDI chip key");
    ui.find("not set").expect("Patch chip placeholder when no patch set");
}

/// A plain (non-external) instrument strip has no `Ext` pill and no chips.
#[test]
fn plain_strip_has_no_ext_pill() {
    let app = app_with_track();
    // No `Enable` — the track stays a plain instrument track.

    let mut ui = simulator(&app);
    assert!(
        ui.find("Ext").is_err(),
        "plain strip must not show the Ext pill"
    );
    assert!(
        ui.find("Patch").is_err(),
        "plain strip must not show the Patch summary chip"
    );
}
