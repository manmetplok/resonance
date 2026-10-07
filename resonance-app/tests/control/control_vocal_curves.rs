//! Vocal expression curves as automation lanes
//! (vocal-mcp-control-and-quality.md §10, slice 14).
//!
//! A curve is stored once per section (in the vocal lane's params),
//! relative to the section start, and reads back at every placement. The
//! `automation.*` methods address it as `{vocal: {track_id, curve}}` with
//! song positions; the app maps them onto placement → section → beat.

use resonance_app::message::{AutomationMessage, Message};
use resonance_app::project::{load_project, save_project};
use resonance_app::Resonance;
use resonance_audio::types::AudioCommand;
use resonance_common::{AutomationTarget, CurveKind, VocalCurveKind};
use resonance_control::methods::automation::{LaneEditResult, LanesResult};
use resonance_control::methods::section as section_proto;
use resonance_control::methods::song::VocalRenderState;
use resonance_control::{ErrorKind, Request};
use serde_json::{json, Value};

use crate::common::roundtrip;
use crate::control_vocal_render_state::{
    app_with_project, call, lane_state, rendered_lane, vocal_view, TRACK,
};

/// A 4-bar section with a vocal lane on `TRACK`, placed at bars 1 and 5.
fn twice_placed_lane(app: &mut Resonance) -> u64 {
    let section_id = call(
        app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 4,
            scale: None,
            place: false,
        },
    )
    .result::<section_proto::CreateResult>()
    .expect("section.create succeeds")
    .section_id;
    for start_bar in [1, 5] {
        call(
            app,
            "section.place",
            &section_proto::PlaceParams {
                definition_id: section_id,
                start_bar,
            },
        )
        .result::<Value>()
        .expect("section.place succeeds");
    }
    let def = u64::from(section_id);
    app.test_install_vocal_lane(def, TRACK);
    def
}

/// Place `def` a second time, at bar 5.
fn place_again(app: &mut Resonance, def: u64) {
    call(
        app,
        "section.place",
        &json!({ "definition_id": def, "start_bar": 5 }),
    )
    .result::<Value>()
    .expect("section.place succeeds");
}

fn target(curve: &str) -> Value {
    json!({ "track_id": TRACK, "curve": curve })
}

fn add_points(app: &mut Resonance, curve: &str, points: Value) -> Result<LaneEditResult, ErrorKind> {
    call(
        app,
        "automation.add_points",
        &json!({ "vocal": target(curve), "points": points }),
    )
    .result::<LaneEditResult>()
    .map_err(|e| e.kind())
}

fn at(bar: u32, beat: f64, value: f64) -> Value {
    json!({ "position": { "bar": bar, "beat": beat }, "value": value })
}

/// `(bar, beat, value)` of every point the lane lists.
fn listed(lane: &resonance_control::methods::automation::LaneView) -> Vec<(u32, f64, f64)> {
    lane.points
        .iter()
        .map(|p| {
            let v = match &p.value {
                resonance_control::methods::automation::AutomationValue::Number(n) => *n,
                other => panic!("a vocal curve value is a number, got {other:?}"),
            };
            (p.position.bar, p.position.beat, v)
        })
        .collect()
}

/// The section's stored curve.
fn stored(app: &Resonance, def: u64, kind: VocalCurveKind) -> Vec<(f64, f32)> {
    let d = app
        .compose_state()
        .definitions
        .iter()
        .find(|d| d.id == def)
        .expect("definition");
    let Some(cfg) = d.lane_generators.get(&TRACK) else {
        return Vec::new();
    };
    let resonance_app::compose::LaneGeneratorKind::Vocal(params) = &cfg.kind else {
        return Vec::new();
    };
    params
        .curves
        .get(kind.index())
        .unwrap()
        .iter()
        .map(|p| (p.beat, p.value))
        .collect()
}

#[test]
fn vocal_lanes_are_listed_section_relative() {
    let mut app = app_with_project();
    twice_placed_lane(&mut app);
    let all: LanesResult = call(&mut app, "automation.lanes", &json!({}))
        .result()
        .expect("automation.lanes succeeds");
    let vocal: Vec<_> = all.lanes.iter().filter(|l| l.target.spec.vocal.is_some()).collect();
    assert_eq!(vocal.len(), 4, "dynamics, tension, breathiness, pitch bend");
    assert!(vocal.iter().all(|l| l.section_relative && l.enabled));

    let one: LanesResult = call(
        &mut app,
        "automation.lanes",
        &json!({ "vocal": target("pitch_bend") }),
    )
    .result()
    .expect("filtered listing");
    assert_eq!(one.lanes.len(), 1);
    let lane = &one.lanes[0];
    assert_eq!(lane.unit, "cents");
    assert_eq!((lane.min, lane.max), (Some(-50.0), Some(50.0)));
    assert_eq!(lane.point_count, 0);
    let wire = serde_json::to_value(lane).unwrap();
    assert_eq!(wire["section_relative"], json!(true));
    assert_eq!(wire["target"]["vocal"], json!({ "track_id": TRACK, "curve": "pitch_bend" }));
}

#[test]
fn a_point_at_one_placement_reads_back_at_every_placement() {
    let mut app = app_with_project();
    let def = twice_placed_lane(&mut app);
    // Bar 6 beat 2 = the second placement, section beat 5.
    let result = add_points(&mut app, "pitch_bend", json!([at(6, 2.0, 25.0)])).expect("written");
    let lane = result.lane.expect("lane reads back");
    assert_eq!(listed(&lane), vec![(2, 2.0, 25.0), (6, 2.0, 25.0)]);
    assert_eq!(stored(&app, def, VocalCurveKind::PitchBend), vec![(5.0, 0.75)]);

    // song.vocal lists the curve on the lane.
    let view = vocal_view(&mut app);
    let wire = serde_json::to_value(&view.lanes[0]).unwrap();
    assert_eq!(
        wire["curves"],
        json!([{ "curve": "pitch_bend", "points": 1, "supported": true }])
    );

    // An upsert at the FIRST placement's copy changes it everywhere.
    let result = add_points(&mut app, "pitch_bend", json!([at(2, 2.0, -10.0)])).expect("upsert");
    assert_eq!(result.replaced, Some(1));
    assert_eq!(
        listed(&result.lane.unwrap()),
        vec![(2, 2.0, -10.0), (6, 2.0, -10.0)]
    );
    assert_eq!(stored(&app, def, VocalCurveKind::PitchBend), vec![(5.0, 0.4)]);
}

#[test]
fn a_point_outside_every_placement_is_refused() {
    let mut app = app_with_project();
    let def = twice_placed_lane(&mut app);
    let revision = app.revision();
    let err = add_points(&mut app, "dynamics", json!([at(10, 1.0, 0.8)]));
    assert_eq!(err.unwrap_err(), ErrorKind::InvalidParams);
    assert!(stored(&app, def, VocalCurveKind::Dynamics).is_empty());
    assert_eq!(app.revision(), revision, "nothing was dispatched");
}

#[test]
fn two_values_for_one_section_beat_are_refused() {
    let mut app = app_with_project();
    twice_placed_lane(&mut app);
    let err = add_points(
        &mut app,
        "dynamics",
        json!([at(2, 1.0, 0.2), at(6, 1.0, 0.9)]),
    );
    assert_eq!(err.unwrap_err(), ErrorKind::InvalidParams);
    // The same value at both copies is one point.
    let ok = add_points(&mut app, "dynamics", json!([at(2, 1.0, 0.2), at(6, 1.0, 0.2)]))
        .expect("agreeing copies are one point");
    assert_eq!(ok.lane.unwrap().point_count, 2);
}

#[test]
fn deleting_one_copy_deletes_the_section_point() {
    let mut app = app_with_project();
    let def = twice_placed_lane(&mut app);
    add_points(&mut app, "tension", json!([at(1, 1.0, 0.2), at(3, 1.0, 0.9)])).expect("written");
    // Projected: bar 1, bar 3, bar 5, bar 7. Delete the copy at bar 7.
    let result: LaneEditResult = call(
        &mut app,
        "automation.delete_points",
        &json!({ "vocal": target("tension"), "indices": [3] }),
    )
    .result()
    .expect("delete succeeds");
    assert_eq!(result.deleted, Some(2), "both copies of the section point");
    assert_eq!(listed(&result.lane.unwrap()), vec![(1, 1.0, 0.2), (5, 1.0, 0.2)]);
    assert_eq!(stored(&app, def, VocalCurveKind::Tension), vec![(0.0, 0.2)]);
}

#[test]
fn one_call_is_one_undo_entry() {
    let mut app = app_with_project();
    let def = twice_placed_lane(&mut app);
    add_points(
        &mut app,
        "breathiness",
        json!([at(1, 1.0, 0.1), at(1, 3.0, 0.5), at(2, 1.0, 0.9)]),
    )
    .expect("written");
    assert_eq!(stored(&app, def, VocalCurveKind::Breathiness).len(), 3);
    roundtrip(&mut app, Request::without_params(9, "edit.undo"))
        .result::<Value>()
        .expect("undo");
    assert!(stored(&app, def, VocalCurveKind::Breathiness).is_empty());
    roundtrip(&mut app, Request::without_params(10, "edit.redo"))
        .result::<Value>()
        .expect("redo");
    assert_eq!(stored(&app, def, VocalCurveKind::Breathiness).len(), 3);
}

#[test]
fn set_lane_replaces_every_section_and_remove_lane_clears_them() {
    let mut app = app_with_project();
    let def = twice_placed_lane(&mut app);
    add_points(&mut app, "dynamics", json!([at(1, 1.0, 0.1), at(2, 1.0, 0.9)])).expect("written");
    let result: LaneEditResult = call(
        &mut app,
        "automation.set_lane",
        &json!({ "vocal": target("dynamics"), "points": [at(7, 1.0, 0.6)] }),
    )
    .result()
    .expect("set_lane succeeds");
    assert_eq!(listed(&result.lane.unwrap()), vec![(3, 1.0, 0.6), (7, 1.0, 0.6)]);
    assert_eq!(stored(&app, def, VocalCurveKind::Dynamics), vec![(8.0, 0.6)]);

    let refused = call(
        &mut app,
        "automation.remove_lane",
        &json!({ "vocal": target("dynamics") }),
    );
    assert_eq!(refused.result::<Value>().unwrap_err().kind(), ErrorKind::NeedsConfirmation);
    call(
        &mut app,
        "automation.remove_lane",
        &json!({ "vocal": target("dynamics"), "confirm": true }),
    )
    .result::<Value>()
    .expect("remove_lane succeeds");
    assert!(stored(&app, def, VocalCurveKind::Dynamics).is_empty());
}

#[test]
fn a_vocal_lane_has_no_read_flag_and_set_params_cannot_write_curves() {
    let mut app = app_with_project();
    let def = twice_placed_lane(&mut app);
    let err = call(
        &mut app,
        "automation.set_enabled",
        &json!({ "vocal": target("dynamics"), "enabled": false }),
    )
    .result::<Value>()
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidParams);

    let err = call(
        &mut app,
        "vocal.set_params",
        &json!({
            "track_id": TRACK,
            "section_id": def,
            "params": { "curves": { "dynamics": [{ "beat": 0.0, "value": 1.0 }] } },
        }),
    )
    .result::<Value>()
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidParams, "curves are not a param");
    assert!(stored(&app, def, VocalCurveKind::Dynamics).is_empty());
}

#[test]
fn a_track_without_a_vocal_lane_is_refused() {
    let mut app = app_with_project();
    let err = add_points(&mut app, "dynamics", json!([at(1, 1.0, 0.5)]));
    assert_eq!(err.unwrap_err(), ErrorKind::InvalidParams);
}

#[test]
fn a_control_curve_edit_makes_the_lane_stale_without_rendering() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let epoch = app.test_vocal_render_epoch(verse, TRACK);
    add_points(&mut app, "pitch_bend", json!([at(1, 1.0, 40.0)])).expect("written");
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Stale);
    assert_eq!(app.test_vocal_render_epoch(verse, TRACK), epoch, "control renders nothing");
    assert_eq!(app.test_vocal_auto_render_pending(verse, TRACK), None);
}

#[test]
fn a_gui_curve_edit_at_placement_two_shows_at_placement_one_and_arms_the_re_render() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    place_again(&mut app, verse);
    let target = AutomationTarget::VocalCurve {
        track: TRACK,
        curve: VocalCurveKind::PitchBend,
    };
    // Bar 6 (0-based bar 5) + a beat: inside the second placement.
    let map = app.test_tempo_map().clone();
    let second = map.bar_to_sample(4);
    let frame = map.bar_to_sample(5) + (map.bar_to_sample(6) - map.bar_to_sample(5)) / 4;
    let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
        target: target.clone(),
        time_frames: frame,
        value: 0.9,
        curve: CurveKind::Linear,
    }));
    let lane = &app.test_automation().vocal.get(&target).expect("vocal row").lane;
    assert_eq!(lane.points.len(), 2, "one section point, two placements");
    let offset = lane.points[1].time_frames - lane.points[0].time_frames;
    assert!(
        offset.abs_diff(second) <= 2,
        "the copies sit one placement apart ({offset} vs {second})"
    );
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Stale);
    assert!(app.test_vocal_auto_render_pending(verse, TRACK).is_some());

    // Outside every placement the row is inert.
    let before = app.compose_state().definitions.clone();
    let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
        target,
        time_frames: map.bar_to_sample(20),
        value: 0.1,
        curve: CurveKind::Linear,
    }));
    assert_eq!(app.compose_state().definitions.len(), before.len());
    assert_eq!(stored(&app, verse, VocalCurveKind::PitchBend).len(), 1);
}

#[test]
fn the_engine_never_receives_a_vocal_curve_lane() {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_install_shipped_voicebanks();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-vocal-curves.rprj"));
    app.test_add_track(TRACK, resonance_audio::types::TrackType::Vocal);
    twice_placed_lane(&mut app);
    add_points(&mut app, "dynamics", json!([at(1, 1.0, 0.8), at(2, 1.0, 0.3)])).expect("written");
    call(
        &mut app,
        "automation.shape",
        &json!({
            "vocal": target("tension"), "start": { "bar": 1 }, "end": { "bar": 3 },
            "shape": "ramp", "from": 0.2, "to": 0.8,
        }),
    )
    .result::<Value>()
    .expect("shape succeeds");
    // And a project load of the same state.
    let dir = std::env::temp_dir().join(format!("resonance-vocal-curves-engine-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create project dir");
    save_project(&dir, &app.test_build_project_file(), &[], &[]).expect("save project");
    let loaded = load_project(&dir).expect("load project");
    app.test_replay_loaded_project_from(loaded);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!app.test_automation().vocal.is_empty() || {
        // The rows are rebuilt by the next update; a read refreshes them.
        call(&mut app, "automation.lanes", &json!({})).result::<Value>().is_ok()
    });
    let leaked: Vec<AudioCommand> = rx
        .try_iter()
        .filter(|c| match c {
            AudioCommand::SetAutomationLane { lane } => !lane.target.is_engine_lane(),
            AudioCommand::ClearAutomationLane { target }
            | AudioCommand::SetAutomationReadEnabled { target, .. } => !target.is_engine_lane(),
            _ => false,
        })
        .collect();
    assert!(leaked.is_empty(), "vocal curve lanes reached the engine: {leaked:?}");
}

fn reopen(app: &Resonance, tag: &str) -> Resonance {
    let dir = std::env::temp_dir().join(format!(
        "resonance-vocal-curves-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create project dir");
    let file = app.test_build_project_file();
    save_project(&dir, &file, &[], &[]).expect("save project");
    let loaded = load_project(&dir).expect("load project");
    let (mut reopened, _task) = Resonance::new_for_test();
    reopened.test_install_shipped_voicebanks();
    reopened.test_replay_loaded_project_from(loaded);
    let _ = std::fs::remove_dir_all(&dir);
    reopened
}

#[test]
fn a_curve_survives_save_reopen_and_an_unrelated_undo() {
    let mut app = app_with_project();
    let def = twice_placed_lane(&mut app);
    add_points(&mut app, "pitch_bend", json!([at(6, 2.0, 25.0)])).expect("written");

    // An unrelated edit, undone: the curve stays.
    call(
        &mut app,
        "transport.set_tempo",
        &json!({ "bpm": 100.0 }),
    )
    .result::<Value>()
    .expect("tempo set");
    roundtrip(&mut app, Request::without_params(9, "edit.undo"))
        .result::<Value>()
        .expect("undo");
    assert_eq!(stored(&app, def, VocalCurveKind::PitchBend), vec![(5.0, 0.75)]);

    let mut reopened = reopen(&app, "save-reopen");
    reopened.test_set_active_project(true);
    assert_eq!(stored(&reopened, def, VocalCurveKind::PitchBend), vec![(5.0, 0.75)]);
    let one: LanesResult = call(
        &mut reopened,
        "automation.lanes",
        &json!({ "vocal": target("pitch_bend") }),
    )
    .result()
    .expect("listing");
    assert_eq!(listed(&one.lanes[0]), vec![(2, 2.0, 25.0), (6, 2.0, 25.0)]);
    // No engine lane was saved for it.
    assert!(reopened.test_build_project_file().automation_lanes.is_empty());
}
