//! Inspector rendering of the external-instrument **Playback Source**
//! toggle (doc #257, todo #1100): structural proofs that the segmented
//! Live/Recorded control renders inside the External Instrument group,
//! that the frost "Playing the recorded take" chip appears only when the
//! mode is `Recorded` *and* a take exists, plus golden-image snapshots of
//! the group in both states (pixel comparison routed through
//! `common::assert_golden`, skipped under `RESONANCE_SKIP_GOLDENS=1`).

mod common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{ExternalInstrumentMessage as Eim, Message, UiMessage};
use resonance_app::state::{TrackState, ViewMode};
use resonance_app::{theme, Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioEvent, TrackId};
use resonance_common::PlaybackSource;

const TRACK: TrackId = 1;
/// Taller than the app default 1440×900 so the full External Instrument
/// group — including the Playback Source block near its bottom and the
/// frost chip under it — fits the inspector rail without scrolling and
/// lands inside the golden pixels.
const WINDOW: (f32, f32) = (1440.0, 1600.0);

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

/// Mixer-pinned app with a configured external-instrument track selected.
fn app_with_external_track() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(TRACK))));

    dispatch(&mut app, Eim::Enable(TRACK));
    dispatch(&mut app, Eim::SetMidiOutDevice(TRACK, Some("Moog Muse".into())));
    dispatch(&mut app, Eim::SetMidiOutChannel(TRACK, Some(0)));
    dispatch(&mut app, Eim::SetReturnDevice(TRACK, Some("Scarlett 18i20".into())));
    app
}

fn dispatch(app: &mut Resonance, m: Eim) {
    let _ = app.update(Message::ExternalInstrument(m));
}

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

/// Land a recorded take on `TRACK` via the real event path — this also
/// exercises the auto-switch to `Recorded`.
fn land_take(app: &mut Resonance) {
    app.test_apply_engine_event(AudioEvent::RecordingFinished {
        clip_id: 42,
        track_id: TRACK,
        start_sample: 0,
        duration_samples: 48_000,
        name: "take 1".into(),
        waveform_peaks: Vec::new(),
    });
}

/// Default (Live) state: the segmented toggle renders inside the group,
/// no frost chip.
#[test]
fn playback_source_toggle_renders_live_state() {
    let app = app_with_external_track();
    let mut ui = simulator(&app);

    ui.find("EXTERNAL INSTRUMENT")
        .expect("External Instrument group header");
    ui.find("PLAYBACK SOURCE").expect("Playback Source field");
    ui.find("Live").expect("Live segment");
    ui.find("Recorded").expect("Recorded segment");
    assert!(
        ui.find("Playing the recorded take").is_err(),
        "no frost chip while Live"
    );

    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(
        &snap,
        "tests/snapshots/external_instrument_playback_source_live.png",
    );
}

/// After a take lands, the mode reads `Recorded` and the frost "Playing
/// the recorded take" chip appears.
#[test]
fn playback_source_recorded_with_take_shows_frost_chip() {
    let mut app = app_with_external_track();
    land_take(&mut app);
    let mut ui = simulator(&app);

    ui.find("PLAYBACK SOURCE").expect("Playback Source field");
    ui.find("Playing the recorded take")
        .expect("frost chip when Recorded with a take");

    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(
        &snap,
        "tests/snapshots/external_instrument_playback_source_recorded.png",
    );
}

/// `Recorded` without any take renders no frost chip — the mode falls
/// back to fully live everywhere, and the toggle alone tells that story.
#[test]
fn playback_source_recorded_without_take_has_no_chip() {
    let mut app = app_with_external_track();
    dispatch(
        &mut app,
        Eim::SetPlaybackSource(TRACK, PlaybackSource::Recorded),
    );
    let mut ui = simulator(&app);

    ui.find("PLAYBACK SOURCE").expect("Playback Source field");
    assert!(
        ui.find("Playing the recorded take").is_err(),
        "no frost chip without a take"
    );
}
