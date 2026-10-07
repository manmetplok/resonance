//! A vocal track's expression-curve rows on the Arrange timeline
//! (vocal-mcp-control-and-quality.md §10, slice 14).
//!
//! Expanding a vocal track shows `Vocal: Dynamics | Tension | Breathiness |
//! Pitch bend` sub-rows. Each placement of a section with a vocal lane is a
//! live band carrying that section's curve; the row outside the placements
//! is dimmed and inert; a placement whose bank does not render the curve
//! (Tension on TIGER) is greyed with "not supported by TIGER" and ignores
//! clicks.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{AutomationMessage, Message, ViewportMessage};
use resonance_app::state::ViewMode;
use resonance_app::view::arrange_layout::ArrangeRowKind;
use resonance_app::{theme, Resonance};
use resonance_audio::types::TrackType;
use resonance_common::{AutomationTarget, CurveKind, VocalCurveKind};
use resonance_control::Request;
use serde_json::{json, Value};

const WINDOW: (f32, f32) = (1440.0, 900.0);
const TRACK: u64 = 40;

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

fn call(app: &mut Resonance, method: &str, params: Value) -> Value {
    common::roundtrip(app, Request::new(1, method, &params).unwrap())
        .result::<Value>()
        .unwrap_or_else(|e| panic!("{method} failed: {e:?}"))
}

/// A 4-bar section with a vocal lane on `TRACK` sung by `bank`, placed at
/// every bar of `bars` (1-based).
fn section(app: &mut Resonance, name: &str, bank: &str, bars: &[u32]) -> u64 {
    let created = call(
        app,
        "section.create",
        json!({ "name": name, "length_bars": 4, "place": false }),
    );
    let def = created["section_id"].as_u64().unwrap();
    for bar in bars {
        call(app, "section.place", json!({ "definition_id": def, "start_bar": bar }));
    }
    app.test_install_vocal_lane(def, TRACK);
    call(
        app,
        "vocal.set_params",
        json!({ "track_id": TRACK, "section_id": def, "params": { "voicebank": bank } }),
    );
    def
}

fn points(app: &mut Resonance, curve: &str, pts: Value) {
    call(
        app,
        "automation.add_points",
        json!({ "vocal": { "track_id": TRACK, "curve": curve }, "points": pts }),
    );
}

fn at(bar: u32, beat: f64, value: f64) -> Value {
    json!({ "position": { "bar": bar, "beat": beat }, "value": value })
}

/// Verse (Lilia) at bars 1 and 13 (bars 9-12 are no placement), Chorus (TIGER) at bar 5, with Dynamics
/// and Tension shaped in both sections; the vocal track expanded.
fn build_app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_install_shipped_voicebanks();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/timeline-vocal-curves.rprj"));
    app.test_add_track(TRACK, TrackType::Vocal);
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportWidth(
        WINDOW.0 - theme::TRACK_HEADER_WIDTH,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(WINDOW.1)));
    section(&mut app, "Verse", "lilia", &[1, 13]);
    section(&mut app, "Chorus", "tiger", &[5]);
    points(
        &mut app,
        "dynamics",
        json!([at(1, 1.0, 0.3), at(2, 3.0, 0.8), at(4, 1.0, 0.5), at(5, 1.0, 0.2), at(7, 1.0, 0.9)]),
    );
    points(&mut app, "tension", json!([at(1, 1.0, 0.7), at(3, 1.0, 0.2), at(6, 1.0, 0.6)]));
    points(&mut app, "pitch_bend", json!([at(2, 1.0, 0.0), at(2, 3.0, 30.0)]));
    let _ = app.update(Message::Automation(AutomationMessage::ToggleTrackExpanded(TRACK)));
    app
}

fn vocal_rows(app: &Resonance) -> usize {
    app.test_arrange_row_layout()
        .rows()
        .iter()
        .filter(|r| matches!(r.kind, ArrangeRowKind::AutomationLane { track, .. } if track == TRACK))
        .count()
}

#[test]
fn a_vocal_track_expands_into_four_curve_rows() {
    let mut app = build_app();
    assert_eq!(vocal_rows(&app), 4);
    let _ = app.update(Message::Automation(AutomationMessage::ToggleTrackExpanded(TRACK)));
    assert_eq!(vocal_rows(&app), 0, "collapsed: no rows");
}

#[test]
fn a_click_on_an_unsupported_placement_is_ignored() {
    let mut app = build_app();
    let tension = AutomationTarget::VocalCurve {
        track: TRACK,
        curve: VocalCurveKind::Tension,
    };
    let before = app.test_automation().vocal.get(&tension).unwrap().lane.points.clone();
    // Bar 6 lies in the Chorus, sung by TIGER, which has no tension input.
    let frame = app.test_tempo_map().bar_to_sample(5) + 1_000;
    let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
        target: tension.clone(),
        time_frames: frame,
        value: 0.9,
        curve: CurveKind::Linear,
    }));
    assert_eq!(app.test_automation().vocal.get(&tension).unwrap().lane.points, before);
}

#[test]
fn vocal_curve_rows_snapshot() {
    let mut app = build_app();
    // Zoomed out far enough to show the gap between placements dimmed.
    for _ in 0..2 {
        let _ = app.update(Message::Viewport(ViewportMessage::ZoomOut));
    }
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/timeline_vocal_curve_rows_expanded.png");
}
