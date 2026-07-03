//! Golden-image snapshot for the Compose drum-pattern **bank manager**
//! modal.
//!
//! The lane-side arrangement editor (the strip that replaced the old
//! single-pattern picker) is covered by `compose_arrangement_strip.rs`;
//! this file locks in the pattern-bank manager modal, which is the
//! cleanest surface for the bank-editing UI.
//!
//! Window size matches the app's default 1440×900 per
//! `ux-guidelines.md`. On first run `matches_image()` writes the
//! goldens under `tests/snapshots/`; subsequent runs diff against the
//! committed PNGs.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::compose::messages::DrumGroupsMessage;
use resonance_app::compose::{ComposeMessage, SelectedLane};
use resonance_app::message::Message;
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

/// Build the demo app pinned to the Compose tab so the drum lane is on
/// screen. The drum lane lives under the section so the focused
/// placement also needs to exist — `seed_demo_content` already does that.
fn build_compose_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);

    // The demo seed lands on the Lead Vocal lane. Snap the lane focus
    // onto the drum track so the picker's "selected" chip + the drum
    // canvas underneath both render in their focused state.
    if let Some(drum_track_id) = app
        .track_registry()
        .tracks
        .iter()
        .find(|t| {
            use resonance_app::state::InstrumentType;
            use resonance_audio::types::TrackType;
            matches!(t.track_type, TrackType::Instrument)
                && t.sub_track.is_none()
                && t.instrument_type == InstrumentType::Drum
        })
        .map(|t| t.id)
    {
        let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
            SelectedLane::Drums(drum_track_id),
        )));
    }

    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

#[test]
fn drum_pattern_manager_modal_lists_patterns() {
    // Opens the Drum Groups Manager modal so the snapshot captures the
    // pattern bank column + the group detail panel for the focused
    // pattern. The pattern picker on the lane itself sits below the
    // viewport at 1440×900 because the synth tracks come first, but
    // the modal sits on top of everything so it's the cleanest place
    // to lock in the pattern-bank UI.
    let mut app = build_compose_app();
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::OpenManager,
    )));
    snapshot_to(
        &app,
        "tests/snapshots/drum_pattern_manager_modal_lists_patterns.png",
    );
}
