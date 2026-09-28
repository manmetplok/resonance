//! `automation.lanes` / `automation.set_lane` (automation-control-api.md
//! slices A2 + A3).
//!
//! The lane round trip in real units, the undo/revision contract (one
//! call → one `SetLane` → one undo entry → one revision), the shared
//! target resolution's errors, the frozen-track rule (D2), meter-aware
//! positions and the documented tempo / meter behaviour.

use crate::common::call;
use resonance_app::message::{AutomationMessage, Message};
use resonance_app::state::{FreezeStatus, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, ParamInfo, ScannedPlugin, TrackType};
use resonance_common::{AutomationTarget, CurveKind, FreezeCacheRef, FreezeCacheStatus};
use resonance_control::methods::automation::{
    AutomationCurve, AutomationValue, LaneControl, LaneEditResult, LaneStatus, LanesResult,
};
use resonance_control::methods::master::MasterSummary;
use resonance_control::methods::song::TracksView;
use resonance_control::{ErrorKind, MutationAck, Request, Response};
use serde_json::json;

const SR: u32 = 48_000;
const TRACK: u64 = 1;
const BUS: u64 = 1_000_001;
const SYNTH: u64 = 10;
const EQ: u64 = 11;
/// One 4/4 bar at 120 BPM, 48 kHz.
const BAR: u64 = 96_000;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    // Undo only records once the project has a path on disk.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-automation.rprj"));
    app.test_set_sample_rate(SR);
    app.test_set_flat_tempo(120.0);
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_add_bus(BUS, "Drums");
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            ScannedPlugin {
                clap_file_path: "/plugins/wavetable.clap".to_owned(),
                clap_plugin_id: "com.resonance.wavetable".to_owned(),
                name: "Wavetable".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: true,
                ..Default::default()
            },
            ScannedPlugin {
                clap_file_path: "/plugins/eq.clap".to_owned(),
                clap_plugin_id: "com.resonance.eq".to_owned(),
                name: "EQ".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
                ..Default::default()
            },
        ],
    });
    mount(
        &mut app,
        SYNTH,
        "com.resonance.wavetable",
        "Wavetable",
        vec![
            ParamInfo {
                id: 7,
                name: "Filter Cutoff".to_owned(),
                min_value: 20.0,
                max_value: 20000.0,
                default_value: 1000.0,
                current_value: 1000.0,
                unit: "Hz".to_owned(),
                ..Default::default()
            },
            ParamInfo {
                id: 8,
                name: "Filter Type".to_owned(),
                min_value: 0.0,
                max_value: 2.0,
                default_value: 0.0,
                current_value: 0.0,
                stepped: true,
                choices: vec![
                    "Low-pass".to_owned(),
                    "Band-pass".to_owned(),
                    "High-pass".to_owned(),
                ],
                ..Default::default()
            },
        ],
    );
    mount(
        &mut app,
        EQ,
        "com.resonance.eq",
        "EQ",
        vec![ParamInfo {
            id: 1,
            name: "Gain".to_owned(),
            min_value: -12.0,
            max_value: 12.0,
            default_value: 0.0,
            current_value: 0.0,
            unit: "dB".to_owned(),
            ..Default::default()
        }],
    );
    app
}

/// Mirror the engine echo that mounts a plugin on [`TRACK`].
fn mount(
    app: &mut Resonance,
    instance_id: u64,
    plugin_id: &str,
    name: &str,
    params: Vec<ParamInfo>,
) {
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id,
        plugin_name: name.to_owned(),
        clap_plugin_id: plugin_id.to_owned(),
        clap_file_path: format!("/plugins/{plugin_id}.clap"),
        params,
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
}

fn set_lane(app: &mut Resonance, params: serde_json::Value) -> Response {
    call(app, "automation.set_lane", params)
}

fn set_lane_ok(app: &mut Resonance, params: serde_json::Value) -> LaneEditResult {
    set_lane(app, params)
        .result()
        .expect("automation.set_lane succeeds")
}

fn lanes(app: &mut Resonance, params: serde_json::Value) -> LanesResult {
    call(app, "automation.lanes", params)
        .result()
        .expect("automation.lanes succeeds")
}

fn error_of(response: Response) -> resonance_control::RpcError {
    response
        .result::<serde_json::Value>()
        .expect_err("the call is refused")
}

fn cutoff_sweep() -> serde_json::Value {
    json!({
        "track_id": TRACK,
        "param": "Filter Cutoff",
        "points": [
            {"position": {"bar": 25}, "value": 4000},
            {"position": {"bar": 17}, "value": 300},
        ],
    })
}

// ---------------------------------------------------------------------------
// Round trip
// ---------------------------------------------------------------------------

#[test]
fn set_lane_reads_back_identically_through_lanes() {
    let mut app = app();
    let written = set_lane_ok(&mut app, cutoff_sweep());
    let lane = written.lane.clone().expect("the reply carries the lane");

    // Sorted, resolved, in real units.
    assert_eq!(lane.status, LaneStatus::Ok);
    assert_eq!(lane.unit, "Hz");
    assert_eq!((lane.min, lane.max), (Some(20.0), Some(20000.0)));
    assert_eq!(lane.target.spec.track_id.map(|t| t.0), Some(TRACK));
    assert_eq!(lane.target.spec.param.as_deref(), Some("Filter Cutoff"));
    assert_eq!(
        lane.target.spec.plugin_id.as_deref(),
        Some("com.resonance.wavetable")
    );
    assert_eq!(lane.target.plugin_name.as_deref(), Some("Wavetable"));
    assert_eq!(lane.target.param_id, Some(7));
    assert_eq!(lane.point_count, 2);
    let p0 = &lane.points[0];
    assert_eq!((p0.index, p0.position.bar, p0.position.beat), (0, 17, 1.0));
    assert_eq!(p0.position.sample, 16 * BAR);
    assert_eq!(p0.value, AutomationValue::Number(300.0));
    assert_eq!(p0.text, "300 Hz");
    assert_eq!(p0.curve, AutomationCurve::Linear);
    let expected_norm = resonance_common::plugin_param_to_lane_value(300.0, 20.0, 20000.0);
    assert_eq!(p0.normalized, f64::from(expected_norm));
    assert_eq!(lane.points[1].value, AutomationValue::Number(4000.0));
    assert_eq!(lane.points[1].position.bar, 25);

    // `automation.lanes` reports the very same lane.
    let read = lanes(&mut app, json!({}));
    assert_eq!(read.lanes, vec![lane.clone()]);
    assert_eq!(read.revision, written.revision);

    // And the model holds what the wire said.
    let target = AutomationTarget::PluginParam {
        instance: SYNTH,
        param_id: 7,
    };
    let model = &app.test_automation().lanes[&target];
    assert_eq!(model.points.len(), 2);
    assert_eq!(model.points[0].time_frames, 16 * BAR);
    assert!(model.enabled);

    // Sending a read-back lane straight back stores the identical values.
    let echo = json!({
        "track_id": TRACK,
        "param": lane.target.spec.param,
        "plugin_id": lane.target.spec.plugin_id,
        "occurrence": lane.target.spec.occurrence,
        "points": lane.points.iter().map(|p| json!({
            "position": {"bar": p.position.bar, "beat": p.position.beat},
            "value": p.value,
            "curve": p.curve,
        })).collect::<Vec<_>>(),
    });
    let again = set_lane_ok(&mut app, echo).lane.expect("lane");
    assert_eq!(again.points, lane.points);
    assert_eq!(again.lane_id, lane.lane_id, "a replace keeps the lane id");
}

#[test]
fn volume_reads_minus_inf_at_the_floor_and_mute_is_stepped_bools() {
    let mut app = app();
    let lane = set_lane_ok(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "points": [
            {"position": {"bar": 1}, "value": "-inf"},
            {"position": {"bar": 2}, "value": -60},
            {"position": {"bar": 3}, "value": -12},
            {"position": {"bar": 4}, "value": 6},
        ]}),
    )
    .lane
    .expect("lane");
    let values: Vec<_> = lane.points.iter().map(|p| p.value.clone()).collect();
    assert_eq!(
        values,
        vec![
            AutomationValue::Text("-inf".into()),
            AutomationValue::Text("-inf".into()),
            AutomationValue::Number(-12.0),
            AutomationValue::Number(6.0),
        ]
    );
    assert_eq!(lane.points[0].normalized, 0.0);
    assert_eq!(lane.points[2].text, "-12 dB");
    assert_eq!(lane.unit, "dB");

    let mute = set_lane_ok(
        &mut app,
        json!({"track_id": TRACK, "control": "mute", "points": [
            {"position": {"bar": 1}, "value": false},
            {"position": {"bar": 5}, "value": 1},
        ]}),
    )
    .lane
    .expect("lane");
    assert_eq!(mute.points[0].value, AutomationValue::Bool(false));
    assert_eq!(mute.points[1].value, AutomationValue::Bool(true));
    assert!(mute
        .points
        .iter()
        .all(|p| p.curve == AutomationCurve::Stepped));

    let pan = set_lane_ok(
        &mut app,
        json!({"track_id": TRACK, "control": "pan", "points": [
            {"position": {"bar": 1}, "value": -0.5},
            {"position": {"bar": 2}, "value": 1},
        ]}),
    )
    .lane
    .expect("lane");
    assert_eq!(pan.points[0].value, AutomationValue::Number(-0.5));
    assert_eq!(pan.points[1].value, AutomationValue::Number(1.0));
}

#[test]
fn a_stepped_parameter_takes_choice_labels_and_reads_them_back_as_text() {
    let mut app = app();
    let lane = set_lane_ok(
        &mut app,
        json!({"track_id": TRACK, "param": "filter type", "points": [
            {"position": {"bar": 1}, "value": "High-pass"},
            {"position": {"bar": 9}, "value": 1},
        ]}),
    )
    .lane
    .expect("lane");
    assert_eq!(lane.points[0].value, AutomationValue::Number(2.0));
    assert_eq!(lane.points[0].text, "High-pass");
    assert_eq!(lane.points[1].text, "Band-pass");
    assert!(
        lane.points
            .iter()
            .all(|p| p.curve == AutomationCurve::Stepped),
        "a stepped parameter defaults to stepped curves"
    );
}

#[test]
fn normalized_values_are_stored_as_given() {
    let mut app = app();
    let lane = set_lane_ok(
        &mut app,
        json!({"track_id": TRACK, "param": "Filter Cutoff", "normalized": true, "points": [
            {"position": {"sample": 0}, "value": 0.5, "curve": "stepped"},
        ]}),
    )
    .lane
    .expect("lane");
    assert_eq!(lane.points[0].normalized, 0.5);
    assert_eq!(lane.points[0].value, AutomationValue::Number(10010.0));
    assert_eq!(lane.points[0].curve, AutomationCurve::Stepped);

    let err = error_of(set_lane(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "normalized": true, "points": [
            {"position": {"sample": 0}, "value": 1.5},
        ]}),
    ));
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
    assert!(err.message.contains("0..=1"), "{}", err.message);
}

#[test]
fn set_lane_replaces_every_point_and_keeps_or_sets_the_read_flag() {
    let mut app = app();
    set_lane_ok(&mut app, cutoff_sweep());
    let lane = set_lane_ok(
        &mut app,
        json!({"track_id": TRACK, "param": "Filter Cutoff", "enabled": false, "points": [
            {"position": {"bar": 3}, "value": 500},
        ]}),
    )
    .lane
    .expect("lane");
    assert_eq!(lane.point_count, 1, "set_lane replaces ALL points");
    assert!(!lane.enabled);

    // An omitted `enabled` keeps the flag.
    let lane = set_lane_ok(
        &mut app,
        json!({"track_id": TRACK, "param": "Filter Cutoff", "points": [
            {"position": {"bar": 4}, "value": 600},
        ]}),
    )
    .lane
    .expect("lane");
    assert!(!lane.enabled);
}

// ---------------------------------------------------------------------------
// Undo / revision
// ---------------------------------------------------------------------------

fn undo(app: &mut Resonance) {
    let _: serde_json::Value =
        crate::common::roundtrip(app, Request::without_params(98, "edit.undo"))
            .result()
            .expect("edit.undo succeeds");
}

fn redo(app: &mut Resonance) {
    let _: serde_json::Value =
        crate::common::roundtrip(app, Request::without_params(97, "edit.redo"))
            .result()
            .expect("edit.redo succeeds");
}

#[test]
fn one_call_is_one_revision_and_one_undo_entry() {
    let mut app = app();
    let before = app.revision();
    let written = set_lane_ok(&mut app, cutoff_sweep());
    assert_eq!(
        written.revision,
        before + 1,
        "one call bumps the revision by exactly one"
    );
    assert_eq!(app.revision(), before + 1);

    let status: resonance_control::methods::edit::EditStatus =
        crate::common::roundtrip(&mut app, Request::without_params(99, "edit.status"))
            .result()
            .expect("edit.status");
    assert_eq!(status.undo_label.as_deref(), Some("automation edit"));
}

#[test]
fn undo_restores_no_lane_and_redo_brings_it_back() {
    let mut app = app();
    let written = set_lane_ok(&mut app, cutoff_sweep()).lane.expect("lane");

    undo(&mut app);
    assert!(
        lanes(&mut app, json!({})).lanes.is_empty(),
        "undo leaves no lane at all"
    );
    assert!(app.test_automation().lanes.is_empty());

    redo(&mut app);
    assert_eq!(lanes(&mut app, json!({})).lanes, vec![written]);
}

#[test]
fn undo_restores_the_previous_points_exactly() {
    let mut app = app();
    let first = set_lane_ok(&mut app, cutoff_sweep()).lane.expect("lane");
    let second = set_lane_ok(
        &mut app,
        json!({"track_id": TRACK, "param": "Filter Cutoff", "points": [
            {"position": {"bar": 2, "beat": 3}, "value": 12345},
        ]}),
    )
    .lane
    .expect("lane");
    assert_ne!(first.points, second.points);

    undo(&mut app);
    assert_eq!(lanes(&mut app, json!({})).lanes, vec![first]);
    redo(&mut app);
    assert_eq!(lanes(&mut app, json!({})).lanes, vec![second]);
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[test]
fn a_bad_param_name_lists_the_valid_names() {
    let mut app = app();
    let err = error_of(set_lane(
        &mut app,
        json!({"track_id": TRACK, "param": "Cutof", "points": [
            {"position": {"bar": 1}, "value": 300},
        ]}),
    ));
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(
        err.message.contains("has no parameter \"Cutof\"")
            && err.message.contains("Filter Cutoff")
            && err.message.contains("Filter Type"),
        "{}",
        err.message
    );
}

#[test]
fn unknown_owners_are_not_found() {
    let mut app = app();
    for spec in [
        json!({"track_id": 77, "control": "volume"}),
        json!({"bus_id": 77, "control": "volume"}),
    ] {
        let mut params = spec.clone();
        params["points"] = json!([{"position": {"bar": 1}, "value": 0}]);
        let err = error_of(set_lane(&mut app, params));
        assert_eq!(err.kind(), ErrorKind::NotFound, "{spec}: {}", err.message);
        let err = error_of(call(&mut app, "automation.lanes", spec.clone()));
        assert_eq!(err.kind(), ErrorKind::NotFound, "{spec}: {}", err.message);
    }
    let err = error_of(set_lane(
        &mut app,
        json!({"track_id": TRACK, "param": "Gain", "plugin_id": "com.resonance.reverb",
               "points": [{"position": {"bar": 1}, "value": 0}]}),
    ));
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(
        err.message.contains("com.resonance.eq"),
        "lists what the track carries: {}",
        err.message
    );
}

#[test]
fn out_of_range_values_name_the_range() {
    let mut app = app();
    let cases = [
        (
            json!({"track_id": TRACK, "control": "volume"}),
            json!(7),
            "-60..=6",
        ),
        (
            json!({"track_id": TRACK, "control": "pan"}),
            json!(-1.5),
            "-1..=1",
        ),
        (
            json!({"track_id": TRACK, "control": "mute"}),
            json!(0.5),
            "true / false",
        ),
        (
            json!({"track_id": TRACK, "param": "Filter Cutoff"}),
            json!(30000),
            "20..=20000",
        ),
        (
            json!({"track_id": TRACK, "param": "Filter Type"}),
            json!("Notch"),
            "High-pass",
        ),
    ];
    for (spec, value, needle) in cases {
        let mut params = spec.clone();
        params["points"] = json!([{"position": {"bar": 1}, "value": value}]);
        let err = error_of(set_lane(&mut app, params));
        assert_eq!(err.kind(), ErrorKind::InvalidParams, "{spec}");
        assert!(
            err.message.contains(needle) && err.message.contains("points[0].value"),
            "{spec}: {}",
            err.message
        );
    }
    assert!(
        app.test_automation().lanes.is_empty(),
        "a refused call writes nothing"
    );
}

#[test]
fn duplicate_frames_and_empty_lists_are_refused() {
    let mut app = app();
    let err = error_of(set_lane(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "points": [
            {"position": {"bar": 2}, "value": 0},
            {"position": {"bar": 3}, "value": 0},
            {"position": {"sample": BAR}, "value": -6},
        ]}),
    ));
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
    assert!(
        err.message.contains("points[0]") && err.message.contains("points[2]"),
        "names both indices: {}",
        err.message
    );

    let err = error_of(set_lane(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "points": []}),
    ));
    assert!(err.message.contains("remove_lane"), "{}", err.message);

    // The per-call limit.
    let many: Vec<_> = (0..2049)
        .map(|i| json!({"position": {"sample": i}, "value": 0}))
        .collect();
    let err = error_of(set_lane(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "points": many}),
    ));
    assert!(
        err.message.contains("2049") && err.message.contains("2048"),
        "{}",
        err.message
    );

    // Bars go through the shared guard.
    let err = error_of(set_lane(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "points": [
            {"position": {"bar": 99_999_999}, "value": 0},
        ]}),
    ));
    assert!(err.message.contains("past the limit"), "{}", err.message);

    // Ambiguous target shapes.
    let err = error_of(set_lane(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "param": "Gain",
               "points": [{"position": {"bar": 1}, "value": 0}]}),
    ));
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
    let err = error_of(set_lane(
        &mut app,
        json!({"master": true, "control": "pan",
               "points": [{"position": {"bar": 1}, "value": 0}]}),
    ));
    assert!(
        err.message.contains("only a volume lane"),
        "{}",
        err.message
    );
    assert!(app.test_automation().lanes.is_empty());
}

// ---------------------------------------------------------------------------
// Frozen tracks (D2)
// ---------------------------------------------------------------------------

fn freeze(app: &mut Resonance) {
    let cache = FreezeCacheRef::new(
        "freeze_1.wav".to_owned(),
        SR,
        32,
        1,
        FreezeCacheStatus::Frozen,
    );
    app.test_set_freeze_status(TRACK, FreezeStatus::Frozen { cache_ref: cache });
}

#[test]
fn a_frozen_track_refuses_plugin_lanes_and_accepts_mixer_lanes() {
    let mut app = app();
    freeze(&mut app);
    let err = error_of(set_lane(&mut app, cutoff_sweep()));
    assert_eq!(err.kind(), ErrorKind::Busy);
    assert!(err.message.contains("frozen"), "{}", err.message);

    let lane = set_lane_ok(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "points": [
            {"position": {"bar": 1}, "value": -6},
        ]}),
    );
    assert!(lane.lane.is_some(), "the mixer stays live while frozen");
    assert!(
        matches!(app.test_freeze_status(TRACK), FreezeStatus::Frozen { .. }),
        "a mixer lane leaves the freeze valid"
    );
}

#[test]
fn the_gui_path_cannot_edit_a_plugin_lane_on_a_frozen_track_either() {
    let mut app = app();
    freeze(&mut app);
    let target = AutomationTarget::PluginParam {
        instance: SYNTH,
        param_id: 7,
    };
    let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
        target: target.clone(),
        time_frames: 0,
        value: 0.5,
        curve: CurveKind::Linear,
    }));
    assert!(
        !app.test_automation().lanes.contains_key(&target),
        "the frozen-input gate drops a plugin-lane edit"
    );
    // A gain lane on the same track is a mixer edit and goes through.
    let _ = app.update(Message::Automation(AutomationMessage::AddLane(
        AutomationTarget::TrackGain(TRACK),
    )));
    assert!(app
        .test_automation()
        .lanes
        .contains_key(&AutomationTarget::TrackGain(TRACK)));
}

// ---------------------------------------------------------------------------
// Positions, tempo and meter
// ---------------------------------------------------------------------------

#[test]
fn positions_are_meter_aware_in_seven_eight() {
    let mut app = app();
    let _: MutationAck = call(
        &mut app,
        "global.add_signature_event",
        json!({"bar": 17, "numerator": 7, "denominator": 8}),
    )
    .result()
    .expect("global.add_signature_event succeeds");

    let lane = set_lane_ok(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "points": [
            {"position": {"bar": 17}, "value": -6},
            {"position": {"bar": 18, "beat": 7.5}, "value": 0},
        ]}),
    )
    .lane
    .expect("lane");
    let read = &lanes(&mut app, json!({"track_id": TRACK})).lanes[0];
    assert_eq!(read, &lane);
    let p = &read.points;
    assert_eq!((p[0].position.bar, p[0].position.beat), (17, 1.0));
    assert_eq!(p[0].position.sample, 16 * BAR);
    // A 7/8 bar at 120 BPM is seven eighths of 12 000 samples.
    let bar_7_8 = 7 * 12_000;
    assert_eq!((p[1].position.bar, p[1].position.beat), (18, 7.5));
    assert_eq!(p[1].position.sample, 16 * BAR + bar_7_8 + 78_000);

    // Beat 8 does not exist in a 7/8 bar.
    let err = error_of(set_lane(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "points": [
            {"position": {"bar": 17, "beat": 8}, "value": 0},
        ]}),
    ));
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
    assert!(
        err.message.contains("points[0].position"),
        "{}",
        err.message
    );
}

#[test]
fn set_tempo_keeps_a_lane_on_its_bar() {
    let mut app = app();
    set_lane_ok(&mut app, cutoff_sweep());
    let _: serde_json::Value = call(&mut app, "transport.set_tempo", json!({"bpm": 60.0}))
        .result()
        .expect("transport.set_tempo succeeds");
    let lane = &lanes(&mut app, json!({})).lanes[0];
    assert_eq!(lane.points[0].position.bar, 17, "re-anchored musically");
    assert_eq!(lane.points[0].position.beat, 1.0);
    assert_eq!(
        lane.points[0].position.sample,
        16 * 2 * BAR,
        "half the tempo, twice the samples"
    );
}

#[test]
fn a_global_tempo_event_keeps_a_lane_on_its_sample() {
    let mut app = app();
    set_lane_ok(&mut app, cutoff_sweep());
    let _: MutationAck = call(
        &mut app,
        "global.add_tempo_event",
        json!({"bar": 9, "bpm": 60.0}),
    )
    .result()
    .expect("global.add_tempo_event succeeds");
    let lane = &lanes(&mut app, json!({})).lanes[0];
    assert_eq!(
        lane.points[0].position.sample,
        16 * BAR,
        "the sample does not move"
    );
    // ...so its BAR does: slower bars from bar 9 on mean fewer of them
    // fit before that sample (how many depends on how the tempo map
    // reaches the new tempo; the point is only that it is no longer 17).
    assert!(
        lane.points[0].position.bar < 17,
        "the point keeps its time, not its bar: {:?}",
        lane.points[0].position
    );
}

// ---------------------------------------------------------------------------
// Listing, filters and the summaries
// ---------------------------------------------------------------------------

#[test]
fn lanes_filter_order_and_window() {
    let mut app = app();
    set_lane_ok(&mut app, cutoff_sweep());
    set_lane_ok(
        &mut app,
        json!({"track_id": TRACK, "param": "Gain", "plugin_id": "com.resonance.eq",
               "points": [{"position": {"bar": 1}, "value": -3}]}),
    );
    set_lane_ok(
        &mut app,
        json!({"track_id": TRACK, "control": "pan",
               "points": [{"position": {"bar": 1}, "value": 0}]}),
    );
    set_lane_ok(
        &mut app,
        json!({"bus_id": BUS, "control": "volume",
               "points": [{"position": {"bar": 1}, "value": -3}]}),
    );
    set_lane_ok(
        &mut app,
        json!({"master": true, "control": "volume",
               "points": [{"position": {"bar": 1}, "value": 0}]}),
    );

    let all = lanes(&mut app, json!({}));
    let order: Vec<(Option<LaneControl>, Option<String>)> = all
        .lanes
        .iter()
        .map(|l| (l.target.spec.control, l.target.param_name.clone()))
        .collect();
    assert_eq!(
        order,
        vec![
            (Some(LaneControl::Pan), None),
            (None, Some("Filter Cutoff".into())),
            (None, Some("Gain".into())),
            (Some(LaneControl::Volume), None), // the bus
            (Some(LaneControl::Volume), None), // the master
        ]
    );
    assert_eq!(all.lanes[3].target.spec.bus_id.map(|b| b.0), Some(BUS));
    assert!(all.lanes[4].target.spec.master);

    assert_eq!(lanes(&mut app, json!({"track_id": TRACK})).lanes.len(), 3);
    assert_eq!(lanes(&mut app, json!({"bus_id": BUS})).lanes.len(), 1);
    assert_eq!(lanes(&mut app, json!({"master": true})).lanes.len(), 1);
    assert_eq!(lanes(&mut app, json!({"control": "volume"})).lanes.len(), 2);
    assert_eq!(
        lanes(&mut app, json!({"param": "filter cutoff"}))
            .lanes
            .len(),
        1
    );
    assert_eq!(lanes(&mut app, json!({"param": "7"})).lanes.len(), 1);
    assert_eq!(
        lanes(&mut app, json!({"plugin_id": "com.resonance.eq"}))
            .lanes
            .len(),
        1
    );

    // A window keeps each point's whole-lane index.
    let windowed = lanes(
        &mut app,
        json!({"param": "Filter Cutoff", "range": {"start": {"bar": 20}}}),
    );
    let lane = &windowed.lanes[0];
    assert_eq!(lane.point_count, 2);
    assert_eq!(lane.points.len(), 1);
    assert_eq!(lane.points[0].index, 1);
}

#[test]
fn an_orphaned_plugin_lane_is_listed_with_its_status() {
    let mut app = app();
    let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
        target: AutomationTarget::PluginParam {
            instance: 999,
            param_id: 3,
        },
        time_frames: 0,
        value: 0.25,
        curve: CurveKind::Linear,
    }));
    let all = lanes(&mut app, json!({}));
    assert_eq!(all.lanes.len(), 1);
    let lane = &all.lanes[0];
    assert_eq!(lane.status, LaneStatus::Orphaned);
    assert_eq!(lane.target.spec.param.as_deref(), Some("3"));
    assert_eq!(lane.target.spec.track_id, None);
    assert_eq!(
        lane.points[0].value,
        AutomationValue::Number(0.25),
        "normalized when unknown"
    );
    // An owner filter never lists an orphan.
    assert!(lanes(&mut app, json!({"track_id": TRACK})).lanes.is_empty());
}

#[test]
fn song_tracks_and_master_summary_carry_the_compact_lists() {
    let mut app = app();
    set_lane_ok(&mut app, cutoff_sweep());
    set_lane_ok(
        &mut app,
        json!({"track_id": TRACK, "control": "volume",
               "points": [{"position": {"bar": 1}, "value": -3}]}),
    );
    set_lane_ok(
        &mut app,
        json!({"master": true, "control": "volume",
               "points": [{"position": {"bar": 1}, "value": 0}]}),
    );

    let view: TracksView = call(&mut app, "song.tracks", json!({}))
        .result()
        .expect("song.tracks");
    let track = view
        .tracks
        .iter()
        .find(|t| t.summary.id.0 == TRACK)
        .expect("track");
    assert_eq!(track.summary.automation_lanes, 2);
    assert_eq!(track.automation.len(), 2);
    assert_eq!(track.automation[0].control, Some(LaneControl::Volume));
    assert_eq!(track.automation[1].param.as_deref(), Some("Filter Cutoff"));
    assert_eq!(
        track.automation[1].plugin_id.as_deref(),
        Some("com.resonance.wavetable")
    );
    assert_eq!(track.automation[1].points, 2);
    assert!(track.automation[1].enabled);

    let master: MasterSummary = call(&mut app, "master.summary", json!(null))
        .result()
        .expect("master.summary");
    assert_eq!(master.automation.len(), 1);
    assert_eq!(master.automation[0].control, Some(LaneControl::Volume));
}

#[test]
fn hello_advertises_the_implemented_automation_methods() {
    let mut app = app();
    let hello: resonance_control::methods::control::HelloResult = call(
        &mut app,
        "control.hello",
        json!({"protocol_version": resonance_control::PROTOCOL_VERSION}),
    )
    .result()
    .expect("control.hello");
    for method in resonance_control::methods::automation::METHODS {
        assert!(
            hello.capabilities.iter().any(|c| c == method),
            "{method} missing"
        );
    }
    assert!(hello.capabilities.iter().any(|c| c == "automation.lanes"));
    assert!(hello
        .capabilities
        .iter()
        .any(|c| c == "automation.set_lane"));
}

#[test]
fn nothing_open_means_busy() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let err = error_of(call(&mut app, "automation.lanes", json!({})));
    assert_eq!(err.kind(), ErrorKind::Busy);
}
