//! Golden-image snapshots for the **unconfigured vocal track
//! placeholder row** in the Compose vocal-lane stack.
//!
//! Commit 40300a4 stopped fabricating default `VocalParams` for vocal
//! tracks without a `Vocal` lane generator, but in doing so made those
//! tracks unreachable: with no row to click there was no way to fire
//! `SelectLane`, so the right-rail generator picker (the only place a
//! vocal generator can be assigned) never opened. Such tracks now
//! render as a compact 64px placeholder row below the configured vocal
//! rows — name column via `lane_side::draw`, dim "No vocal generator —
//! select to set up" hint, shared bar grid, warm wash when selected.
//! Three states are locked in:
//!
//! 1. **Unselected placeholder** — demo content plus an extra
//!    unconfigured "Backing Vox" vocal track: the placeholder renders
//!    below the configured Lead Vocal row, grid-aligned, no wash.
//! 2. **Selected placeholder** — after
//!    `ComposeMessage::SelectLane(SelectedLane::Instrument(..))` the
//!    row shows the warm selection wash and the right rail shows the
//!    generator picker for the unconfigured track.
//! 3. **All vocal tracks unconfigured** — the Lead Vocal config is
//!    removed; the stack must still render the placeholder rather than
//!    collapsing to 0 height.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::compose::{ComposeMessage, SelectedLane};
use resonance_app::message::Message;
use resonance_app::state::{TrackState, ViewMode};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

/// Window size matches the app's default & minimum window per the
/// design guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// Demo fixture's Lead Vocal track (pre-seeded with a Vocal lane
/// generator by `seed_demo_content`).
const LEAD_VOCAL_ID: u64 = 6;
/// Extra vocal track added by these tests, deliberately left without a
/// lane-generator config so it exercises the placeholder row.
const BACKING_VOCAL_ID: u64 = 7;

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

/// Build the demo app pinned to the Compose tab, with one extra
/// unconfigured vocal track appended after the demo's configured one.
fn build_app_with_unconfigured_vocal() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);

    let mut backing = TrackState::new_vocal(BACKING_VOCAL_ID, 6);
    backing.name = "Backing Vox".to_string();
    app.test_push_track(backing);
    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(
        sim_settings(),
        Size::new(WINDOW.0, WINDOW.1),
        app.view(),
    );
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

/// Unselected placeholder — compact row below the configured Lead
/// Vocal lane: MELODY pill + "Backing Vox" in the name column, dim
/// setup hint in the body, bar grid aligned with the lanes above.
#[test]
fn compose_vocal_placeholder_unconfigured_row() {
    let app = build_app_with_unconfigured_vocal();
    snapshot_to(
        &app,
        "tests/snapshots/compose_vocal_placeholder_unconfigured_row.png",
    );
}

/// Selecting the placeholder via the real `SelectLane` reducer path
/// must paint the warm selection wash on the row and open the
/// right-rail inspector for the unconfigured track (generator picker).
#[test]
fn compose_vocal_placeholder_selected() {
    let mut app = build_app_with_unconfigured_vocal();
    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
        SelectedLane::Instrument(BACKING_VOCAL_ID),
    )));
    snapshot_to(
        &app,
        "tests/snapshots/compose_vocal_placeholder_selected.png",
    );
}

/// The point of the placeholder is that the user can wire the track up
/// from the inspector it opens: picking "Vocal" in the generator
/// picker must install a `LaneGeneratorKind::Vocal` config and replace
/// the placeholder with a full lyric/contour row. The pick_list's
/// closed-state label isn't reachable through `iced_test`'s text
/// selector, so this drives the exact message the picker's `on_select`
/// closure emits, through the real reducer, then pins a golden of the
/// now-configured row.
#[test]
fn compose_vocal_placeholder_wires_up_via_picker() {
    use resonance_app::compose::messages::LaneInspectorMsg;
    use resonance_app::compose::LaneGeneratorKindTag;

    let mut app = build_app_with_unconfigured_vocal();
    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
        SelectedLane::Instrument(BACKING_VOCAL_ID),
    )));

    let definition_id = app.compose_state().definitions[0].id;
    let _ = app.update(Message::Compose(ComposeMessage::LaneInspector {
        definition_id,
        track_id: BACKING_VOCAL_ID,
        msg: LaneInspectorMsg::SetGenerator(LaneGeneratorKindTag::Vocal),
    }));

    let def = &app.compose_state().definitions[0];
    assert!(
        matches!(
            def.lane_generators.get(&BACKING_VOCAL_ID).map(|c| &c.kind),
            Some(resonance_app::compose::LaneGeneratorKind::Vocal(_))
        ),
        "picking Vocal in the generator picker must install a Vocal lane generator"
    );

    snapshot_to(
        &app,
        "tests/snapshots/compose_vocal_placeholder_wired_up.png",
    );
}

/// Edge case: every vocal track unconfigured. Removing the demo's
/// pre-seeded Lead Vocal generator must not collapse the vocal stack
/// to 0 height — the placeholder row alone keeps the track reachable.
#[test]
fn compose_vocal_all_unconfigured() {
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    app.test_remove_lane_generator(LEAD_VOCAL_ID);
    snapshot_to(
        &app,
        "tests/snapshots/compose_vocal_all_unconfigured.png",
    );
}
