//! Golden-image snapshot for the External-Instrument **patch-by-name** picker
//! (ba todo #725, architecture doc #201 §5, epic #40).
//!
//! Locks in the named-patch Patch card: with the bundled Moog Muse preset
//! selected on an external-instrument track, the numeric Bank/Program pickers
//! are replaced by a single named-patch picker (grouped by bank/category),
//! the numeric tiles read out the resolved bank/program, and the selected
//! patch's group is shown beside the program tile.
//!
//! Window size matches the app's default 1440×900. On first run
//! `matches_image()` writes the golden under `tests/snapshots/`; subsequent
//! runs diff against the committed PNG.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{ExternalInstrumentMessage as Eim, Message, UiMessage};
use resonance_app::state::{TrackState, ViewMode};
use resonance_app::{theme, Resonance, STARTUP_TAB};
use resonance_audio::types::TrackId;

const TRACK: TrackId = 1;
const WINDOW: (f32, f32) = (1440.0, 900.0);
const MUSE: &str = "moog-muse";

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

/// Mixer-pinned app with a single external-instrument track that has the
/// bundled Moog Muse preset selected and a named factory patch chosen, so the
/// inspector renders the named-patch Patch card.
fn build_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(TRACK))));

    let dispatch = |app: &mut Resonance, m: Eim| {
        let _ = app.update(Message::ExternalInstrument(m));
    };
    dispatch(&mut app, Eim::Enable(TRACK));
    dispatch(&mut app, Eim::SetMidiOutDevice(TRACK, Some("Moog Muse".into())));
    dispatch(&mut app, Eim::SetMidiOutChannel(TRACK, Some(0)));
    dispatch(&mut app, Eim::SetReturnDevice(TRACK, Some("Scarlett 18i20".into())));
    dispatch(&mut app, Eim::SetDevice(TRACK, Some(MUSE.to_string())));
    // "Bank 2 · Patch 1" → combined bank 1, program 0.
    dispatch(&mut app, Eim::SetPatch(TRACK, Some(1), Some(0)));
    dispatch(&mut app, Eim::ToggleMonitor(TRACK));
    app
}

#[test]
fn named_patch_picker_render() {
    let app = build_app();
    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image("tests/snapshots/external_instrument_named_patch_picker.png")
            .expect("matches_image i/o"),
        "snapshot diverged from golden"
    );
}
