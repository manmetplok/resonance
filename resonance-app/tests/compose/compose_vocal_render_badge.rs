//! Golden-image snapshots of the vocal lane's render-state badge
//! (vocal-mcp-control-and-quality.md §7, D22): the lane inspector shows a
//! warm STALE card once the lane's audio no longer matches its content,
//! and a BAD card with the failure's message after a failed render; the
//! vocal-roll toolbar carries the same chip on one line.
//!
//! Each state is reached through the real paths — a GUI re-render landed
//! with `test_finish_vocal_render`, then a GUI lyric edit (stale) or a
//! re-render that fails (error) — and its label is asserted as text
//! before the pixels.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::compose::messages::LaneInspectorMsg;
use resonance_app::compose::{
    ComposeMessage, LaneGeneratorKind, RailPanelKey, SelectedLane, VocalLaneRenderState,
};
use resonance_app::message::{Message, MidiEditorMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};

const WINDOW: (f32, f32) = (1440.0, 900.0);
/// Demo Lead Vocal track and its derived melody clip.
const LEAD_VOCAL: u64 = 6;
const VOCAL_CLIP: u64 = 16;

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

fn snapshot(app: &Resonance, path: &str, expect_text: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    assert!(ui.find(expect_text).is_ok(), "the badge reads {expect_text:?}");
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

fn lane_msg(app: &mut Resonance, def: u64, msg: LaneInspectorMsg) {
    let _ = app.update(Message::Compose(ComposeMessage::LaneInspector {
        definition_id: def,
        track_id: LEAD_VOCAL,
        msg,
    }));
}

/// The demo on the Compose tab with the Lead Vocal lane selected, its
/// draft and melody cards folded (the badge sits at the top of the rail),
/// and a GUI re-render of it landed: a `rendered` lane.
fn rendered_demo() -> (Resonance, u64) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    demo::seed_demo_content(&mut app);
    app.test_set_active_project(true);
    app.test_install_shipped_voicebanks();
    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
        SelectedLane::Instrument(LEAD_VOCAL),
    )));
    for key in [RailPanelKey::VocalDraft(LEAD_VOCAL), RailPanelKey::VocalMelody(LEAD_VOCAL)] {
        let _ = app.update(Message::Compose(ComposeMessage::ToggleRailPanel(key)));
    }
    let def = app
        .compose_state()
        .selected_placement()
        .expect("demo selects a placement")
        .definition_id;
    lane_msg(&mut app, def, LaneInspectorMsg::RerenderVocalAudio);
    assert!(app.test_finish_vocal_render(def, LEAD_VOCAL), "the re-render was queued");
    let status = app.test_vocal_lane_render_status(def, LEAD_VOCAL);
    assert_eq!(status.state, VocalLaneRenderState::Rendered);
    (app, def)
}

/// Edit the lane's first lyric line from the inspector: its audio goes
/// stale (and a debounced re-render is armed, which never runs here).
fn edit_first_line(app: &mut Resonance, def: u64) {
    let n = match &app.compose_state().find_definition(def).unwrap().lane_generators
        [&LEAD_VOCAL]
        .kind
    {
        LaneGeneratorKind::Vocal(p) => p.draft[0].n,
        _ => panic!("a vocal lane"),
    };
    lane_msg(app, def, LaneInspectorMsg::SetVocalLineText(n, "something else entirely".into()));
}

fn stale_label(app: &Resonance, def: u64) -> String {
    let status = app.test_vocal_lane_render_status(def, LEAD_VOCAL);
    assert_eq!(status.state, VocalLaneRenderState::Stale);
    assert!(status.stale_units > 0);
    format!(
        "STALE \u{00B7} {} of {} phrase{} changed",
        status.stale_units,
        status.total_units,
        if status.total_units == 1 { "" } else { "s" }
    )
}

#[test]
fn vocal_rail_stale_badge() {
    let (mut app, def) = rendered_demo();
    edit_first_line(&mut app, def);
    let label = stale_label(&app, def);
    snapshot(&app, "tests/snapshots/compose_vocal_rail_stale_badge.png", &label);
}

#[test]
fn vocal_rail_error_badge() {
    // A render that fails off-thread. (A missing bank's message names the
    // hermetic voicebank folder, whose path differs per run.)
    let (mut app, def) = rendered_demo();
    lane_msg(&mut app, def, LaneInspectorMsg::RerenderVocalAudio);
    assert!(app.test_fail_vocal_render(
        def,
        LEAD_VOCAL,
        resonance_app::compose::vocal_svs::VocalRenderErrorKind::Failed,
        "SVS render: the acoustic model returned no frames",
    ));
    let status = app.test_vocal_lane_render_status(def, LEAD_VOCAL);
    assert_eq!(status.state, VocalLaneRenderState::Error);
    snapshot(
        &app,
        "tests/snapshots/compose_vocal_rail_error_badge.png",
        "RENDER FAILED \u{00B7} render_failed",
    );
}

#[test]
fn vocal_roll_toolbar_stale_badge() {
    let (mut app, def) = rendered_demo();
    let _ = app.update(Message::MidiEditor(MidiEditorMessage::OpenMidiEditor(VOCAL_CLIP)));
    edit_first_line(&mut app, def);
    let label = stale_label(&app, def);
    snapshot(&app, "tests/snapshots/compose_vocal_roll_stale_badge.png", &label);
}
