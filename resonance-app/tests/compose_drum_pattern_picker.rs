//! Golden-image snapshots for the Compose drum lane's pattern picker.
//!
//! Three states are locked in:
//!
//! 1. **default-pattern** — section uses the default "Main" pattern.
//!    Picker shows the "Main" chip with the warm-tint border.
//! 2. **assigned-b-section** — after `AssignPattern` to the second
//!    bank entry the picker chip switches and the lane underneath
//!    re-renders against the new pattern's (empty) group list.
//! 3. **renaming** — `BeginRenamePattern` swaps the chip for a
//!    `text_input`. Locks in the rename affordance.
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
/// Taller window used for the drum-lane snapshots so the picker chip
/// row and the drum canvas sit inside the viewport. 1440×900 is too
/// short to fit all the synth lanes + the drum lane on one screen.
const TALL_WINDOW: (f32, f32) = (1440.0, 1600.0);

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
    snapshot_to_window(app, path, WINDOW);
}

fn snapshot_to_window(app: &Resonance, path: &str, window: (f32, f32)) {
    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(window.0, window.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

#[test]
fn drum_pattern_picker_default() {
    let app = build_compose_app();
    snapshot_to_window(
        &app,
        "tests/snapshots/drum_pattern_picker_default.png",
        TALL_WINDOW,
    );
}

#[test]
fn drum_pattern_picker_assigned_b_section() {
    let mut app = build_compose_app();
    // The seeded bank has two patterns ("Main" + "B section"). Pick the
    // second so the lane swaps to the empty groups list.
    let assignment = {
        let compose = app.compose_state();
        let definition_id = compose.selected_placement().unwrap().definition_id;
        let pattern_id = compose.drum_patterns.get(1).expect("two patterns").id;
        (definition_id, pattern_id)
    };
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::AssignPattern {
            definition_id: assignment.0,
            pattern_id: Some(assignment.1),
        },
    )));
    snapshot_to_window(
        &app,
        "tests/snapshots/drum_pattern_picker_assigned_b_section.png",
        TALL_WINDOW,
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

#[test]
fn drum_pattern_picker_renaming() {
    let mut app = build_compose_app();
    let pattern_id = app
        .compose_state()
        .drum_patterns
        .first()
        .map(|p| p.id)
        .expect("demo seeds at least one pattern");
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::BeginRenamePattern { pattern_id },
    )));
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::UpdateRenamePatternText("Verse Drums".to_string()),
    )));
    snapshot_to_window(
        &app,
        "tests/snapshots/drum_pattern_picker_renaming.png",
        TALL_WINDOW,
    );
}
