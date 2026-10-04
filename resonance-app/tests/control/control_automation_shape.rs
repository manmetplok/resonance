//! `automation.shape` (automation-control-api.md slice A6, §3 D3, §4.6).
//!
//! Point counts per shape, the end point AT `end` holding `to` (D3), the
//! closed-range splice that keeps points outside `[start, end]`, the
//! target rules (exp refused on dB, mute and stepped parameters forced
//! stepped), meter-aware per-bar density, the seed echo, the per-call
//! limit's error and the one-call / one-undo-entry contract.

use crate::common::call;
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{ChainOwner, AudioEvent, ParamInfo, ScannedPlugin, TrackType};
use resonance_control::methods::automation::{
    AutomationCurve, AutomationValue, LaneEditResult, LaneView, LanesResult,
};
use resonance_control::{ErrorKind, MutationAck, Request, Response};
use serde_json::json;

const SR: u32 = 48_000;
const TRACK: u64 = 1;
const SYNTH: u64 = 10;
/// One 4/4 bar at 120 BPM, 48 kHz.
const BAR: u64 = 96_000;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    // Undo only records once the project has a path on disk.
    app.test_set_project_path(std::path::PathBuf::from(
        "/tmp/control-automation-shape.rprj",
    ));
    app.test_set_sample_rate(SR);
    app.test_set_flat_tempo(120.0);
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![ScannedPlugin {
            clap_file_path: "/plugins/wavetable.clap".to_owned(),
            clap_plugin_id: "com.resonance.wavetable".to_owned(),
            name: "Wavetable".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: true,
            ..Default::default()
        }],
    });
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        owner: ChainOwner::Track(TRACK),
        instance_id: SYNTH,
        plugin_name: "Wavetable".to_owned(),
        clap_plugin_id: "com.resonance.wavetable".to_owned(),
        clap_file_path: "/plugins/com.resonance.wavetable.clap".to_owned(),
        params: vec![
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
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
    app
}

fn shape(app: &mut Resonance, params: serde_json::Value) -> Response {
    call(app, "automation.shape", params)
}

fn shape_ok(app: &mut Resonance, params: serde_json::Value) -> LaneEditResult {
    shape(app, params)
        .result()
        .expect("automation.shape succeeds")
}

fn lane_of(result: LaneEditResult) -> LaneView {
    result.lane.expect("the reply carries the lane")
}

fn lanes(app: &mut Resonance) -> LanesResult {
    call(app, "automation.lanes", json!({}))
        .result()
        .expect("automation.lanes succeeds")
}

fn error_of(response: Response) -> resonance_control::RpcError {
    response
        .result::<serde_json::Value>()
        .expect_err("the call is refused")
}

fn number(value: &AutomationValue) -> f64 {
    match value {
        AutomationValue::Number(n) => *n,
        other => panic!("expected a number, got {other:?}"),
    }
}

/// The §5 worked example, on this test's track.
fn cutoff_example() -> serde_json::Value {
    json!({"track_id": TRACK, "param": "Filter Cutoff", "start": {"bar": 17},
           "end": {"bar": 25}, "shape": "exp", "from": 300, "to": 4000})
}

// ---------------------------------------------------------------------------
// Point counts and the end point (D3)
// ---------------------------------------------------------------------------

#[test]
fn every_shape_has_its_point_count_and_a_point_at_end() {
    // Over bars 1-2 (end {bar: 3}): 2 bars. The sweeps land on `to` (0 dB)
    // at end; the oscillators end on their own phase, which after whole
    // cycles is `from` (-30 dB).
    for (shape_name, expected, end_value) in [
        ("ramp", 2, 0.0),
        ("sine", 16 * 2 + 1, -30.0),
        ("triangle", 3, -30.0),
        ("square", 3, -30.0),
        ("steps", 2 + 1, 0.0),
        ("random_walk", 4 * 2 + 1, 0.0),
    ] {
        let mut app = app();
        let lane = lane_of(shape_ok(
            &mut app,
            json!({"track_id": TRACK, "control": "volume", "start": {"bar": 1},
                   "end": {"bar": 3}, "shape": shape_name, "from": -30, "to": 0}),
        ));
        assert_eq!(lane.points.len(), expected, "{shape_name} point count");
        let first = &lane.points[0];
        assert_eq!(first.position.sample, 0, "{shape_name} starts at start");
        let last = lane.points.last().unwrap();
        assert_eq!(last.position.sample, 2 * BAR, "{shape_name} ends AT end");
        assert_eq!(
            (last.position.bar, last.position.beat),
            (3, 1.0),
            "{shape_name}"
        );
        assert_eq!(
            last.value,
            AutomationValue::Number(end_value),
            "{shape_name} end value"
        );
        for w in lane.points.windows(2) {
            assert!(w[0].position.sample < w[1].position.sample, "{shape_name}");
        }
    }
}

#[test]
fn the_worked_example_sweeps_the_cutoff_exponentially_to_4000() {
    let mut app = app();
    let lane = lane_of(shape_ok(&mut app, cutoff_example()));
    // 8 bars (17-24) at 16 per bar, plus the point at bar 25.
    assert_eq!(lane.points.len(), 8 * 16 + 1);
    let first = &lane.points[0];
    assert_eq!(first.position.sample, 16 * BAR);
    assert_eq!(first.value, AutomationValue::Number(300.0));
    let last = lane.points.last().unwrap();
    assert_eq!(last.position.sample, 24 * BAR);
    assert_eq!(last.value, AutomationValue::Number(4000.0));
    assert_eq!(last.text, "4000 Hz");
    for w in lane.points.windows(2) {
        assert!(
            number(&w[1].value) > number(&w[0].value),
            "monotonic: {:?} then {:?}",
            w[0],
            w[1]
        );
        assert!(w[1].normalized > w[0].normalized);
    }
    // Halfway in time (bar 21) is the geometric mean, not the linear one.
    let mid = lane
        .points
        .iter()
        .find(|p| p.position.sample == 20 * BAR)
        .expect("a point on bar 21");
    let geometric = (300.0f64 * 4000.0).sqrt();
    assert!(
        (number(&mid.value) - geometric).abs() < 0.5,
        "bar 21 reads {:?}, expected ~{geometric}",
        mid.value
    );
    // It reads back identically.
    assert_eq!(lanes(&mut app).lanes, vec![lane]);
}

#[test]
fn a_seven_eight_bar_gets_as_many_points_as_a_four_four_bar() {
    let mut app = app();
    for (bar, numerator, denominator) in [(17, 7, 8), (18, 4, 4)] {
        let _: MutationAck = call(
            &mut app,
            "global.add_signature_event",
            json!({"bar": bar, "numerator": numerator, "denominator": denominator}),
        )
        .result()
        .expect("global.add_signature_event succeeds");
    }
    let lane = lane_of(shape_ok(
        &mut app,
        json!({"track_id": TRACK, "param": "Filter Cutoff", "start": {"bar": 17},
               "end": {"bar": 19}, "shape": "exp", "from": 300, "to": 4000}),
    ));
    let in_bar = |bar: u32| lane.points.iter().filter(|p| p.position.bar == bar).count();
    assert_eq!(in_bar(17), 16, "the 7/8 bar");
    assert_eq!(in_bar(18), 16, "the 4/4 bar");
    assert_eq!(in_bar(19), 1, "the end point");
    // The 7/8 bar (seven eighths of 12 000 samples) is shorter in time,
    // so it covers less of the sweep than the 4/4 bar after it: the value
    // at bar 18's downbeat is 7/15 of the way through, geometrically.
    let bar_18 = lane
        .points
        .iter()
        .find(|p| p.position.bar == 18 && p.position.beat == 1.0)
        .expect("a point on bar 18's downbeat");
    assert_eq!(bar_18.position.sample, 16 * BAR + 7 * 12_000);
    let expected = 300.0 * (4000.0f64 / 300.0).powf(7.0 / 15.0);
    assert!(
        (number(&bar_18.value) - expected).abs() < 0.5,
        "bar 18 reads {:?}, expected ~{expected}",
        bar_18.value
    );
}

// ---------------------------------------------------------------------------
// The splice: [start, end] replaced, the rest kept
// ---------------------------------------------------------------------------

#[test]
fn points_inside_the_closed_range_are_replaced_and_the_rest_kept() {
    let mut app = app();
    let _: LaneEditResult = call(
        &mut app,
        "automation.set_lane",
        json!({"track_id": TRACK, "control": "volume", "points": [
            {"position": {"bar": 1}, "value": -12},
            {"position": {"bar": 2}, "value": -11},
            {"position": {"bar": 2, "beat": 3}, "value": -10},
            {"position": {"bar": 3}, "value": -9},
            {"position": {"bar": 5}, "value": -8},
        ]}),
    )
    .result()
    .expect("automation.set_lane succeeds");

    let lane = lane_of(shape_ok(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "start": {"bar": 2},
               "end": {"bar": 3}, "shape": "ramp", "from": -40, "to": -3}),
    ));
    let got: Vec<(u64, AutomationValue)> = lane
        .points
        .iter()
        .map(|p| (p.position.sample, p.value.clone()))
        .collect();
    assert_eq!(
        got,
        vec![
            (0, AutomationValue::Number(-12.0)),
            (BAR, AutomationValue::Number(-40.0)),
            (2 * BAR, AutomationValue::Number(-3.0)),
            (4 * BAR, AutomationValue::Number(-8.0)),
        ],
        "both edge points and the one inside are replaced; bars 1 and 5 are kept"
    );
}

// ---------------------------------------------------------------------------
// Target rules
// ---------------------------------------------------------------------------

#[test]
fn exp_on_volume_is_refused_and_writes_nothing() {
    let mut app = app();
    let before = app.revision();
    let err = error_of(shape(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "start": {"bar": 1},
               "end": {"bar": 3}, "shape": "exp", "from": -30, "to": -3}),
    ));
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
    assert!(
        err.message.contains("dB is already logarithmic; use ramp"),
        "{}",
        err.message
    );
    assert!(app.test_automation().lanes.is_empty());
    assert_eq!(app.revision(), before);
}

#[test]
fn a_volume_shape_accepts_minus_inf() {
    let mut app = app();
    let lane = lane_of(shape_ok(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "start": {"bar": 1},
               "end": {"bar": 5}, "shape": "ramp", "from": "-inf", "to": 0}),
    ));
    assert_eq!(
        lane.points[0].value,
        AutomationValue::Text("-inf".to_owned())
    );
    assert_eq!(lane.points[1].value, AutomationValue::Number(0.0));
}

#[test]
fn a_mute_shape_is_stepped_bools() {
    let mut app = app();
    let lane = lane_of(shape_ok(
        &mut app,
        json!({"track_id": TRACK, "control": "mute", "start": {"bar": 1},
               "end": {"bar": 3}, "shape": "sine", "from": false, "to": true,
               "resolution": 4}),
    ));
    assert_eq!(lane.points.len(), 4 * 2 + 1);
    for p in &lane.points {
        assert_eq!(p.curve, AutomationCurve::Stepped, "{p:?}");
        assert!(matches!(p.value, AutomationValue::Bool(_)), "{p:?}");
        assert!(p.normalized == 0.0 || p.normalized == 1.0, "{p:?}");
    }
    // A whole-cycle sine swings to `to` mid-way and ends back on `from`.
    assert!(lane
        .points
        .iter()
        .any(|p| p.value == AutomationValue::Bool(true)));
    assert_eq!(
        lane.points.last().unwrap().value,
        AutomationValue::Bool(false)
    );
    // A ramp, a sweep, lands on `to`.
    let ramp = lane_of(shape_ok(
        &mut app,
        json!({"track_id": TRACK, "control": "mute", "start": {"bar": 5},
               "end": {"bar": 6}, "shape": "ramp", "from": false, "to": true}),
    ));
    let last = ramp.points.last().unwrap();
    assert_eq!(last.value, AutomationValue::Bool(true));
    assert_eq!(last.curve, AutomationCurve::Stepped);
}

#[test]
fn a_stepped_parameter_takes_labels_and_is_rounded_to_its_steps() {
    let mut app = app();
    let lane = lane_of(shape_ok(
        &mut app,
        json!({"track_id": TRACK, "param": "Filter Type", "start": {"bar": 1},
               "end": {"bar": 2}, "shape": "sine", "from": "Low-pass", "to": "High-pass",
               "resolution": 8}),
    ));
    assert_eq!(lane.points.len(), 8 + 1);
    for p in &lane.points {
        assert_eq!(p.curve, AutomationCurve::Stepped, "{p:?}");
        let v = number(&p.value);
        assert_eq!(v, v.round(), "{p:?} is on a step");
    }
    assert!(lane.points.iter().any(|p| p.text == "High-pass"));
    assert_eq!(lane.points.last().unwrap().text, "Low-pass");
}

#[test]
fn a_range_starting_and_ending_mid_bar_keeps_the_density_uniform() {
    let mut app = app();
    // Bar 1 beat 3 to bar 3 beat 2: half a bar, a whole bar, a quarter.
    let lane = lane_of(shape_ok(
        &mut app,
        json!({"track_id": TRACK, "param": "Filter Cutoff",
               "start": {"bar": 1, "beat": 3}, "end": {"bar": 3, "beat": 2},
               "shape": "exp", "from": 300, "to": 4000}),
    ));
    assert_eq!(lane.points.len(), 8 + 16 + 4 + 1);
    // Every step is a sixteenth of a bar.
    for w in lane.points.windows(2) {
        assert_eq!(
            w[1].position.sample - w[0].position.sample,
            BAR / 16,
            "{:?} then {:?}",
            w[0].position,
            w[1].position
        );
    }
    assert_eq!(lane.points[0].position.sample, BAR / 2);
    let last = lane.points.last().unwrap();
    assert_eq!(last.position.sample, 2 * BAR + BAR / 4);
    assert_eq!(last.value, AutomationValue::Number(4000.0));
}

// ---------------------------------------------------------------------------
// random_walk
// ---------------------------------------------------------------------------

#[test]
fn random_walk_echoes_its_seed_and_is_deterministic() {
    let walk = |seed: Option<u64>| {
        let mut app = app();
        let mut params = json!({"track_id": TRACK, "param": "Filter Cutoff",
            "start": {"bar": 1}, "end": {"bar": 5}, "shape": "random_walk",
            "from": 500, "to": 2000});
        if let Some(seed) = seed {
            params["seed"] = json!(seed);
        }
        shape_ok(&mut app, params)
    };
    let a = walk(Some(42));
    let b = walk(Some(42));
    assert_eq!(a.seed, Some(42));
    assert_eq!(a.lane, b.lane, "same seed, same walk");
    let c = walk(Some(43));
    assert_ne!(a.lane, c.lane, "another seed, another walk");
    assert_eq!(walk(None).seed, Some(0), "the default seed is echoed");

    let lane = a.lane.expect("lane");
    for p in &lane.points {
        let v = number(&p.value);
        assert!((500.0..=2000.0).contains(&v), "{p:?} within from..to");
    }

    let mut app = app();
    let ramp = shape_ok(
        &mut app,
        json!({"track_id": TRACK, "control": "pan", "start": {"bar": 1},
               "end": {"bar": 2}, "shape": "ramp", "from": -1, "to": 1}),
    );
    assert_eq!(ramp.seed, None, "only random_walk echoes a seed");
}

// ---------------------------------------------------------------------------
// Undo / revision
// ---------------------------------------------------------------------------

#[test]
fn one_call_is_one_revision_and_undo_restores_exactly() {
    let mut app = app();
    let _: LaneEditResult = call(
        &mut app,
        "automation.set_lane",
        json!({"track_id": TRACK, "param": "Filter Cutoff", "points": [
            {"position": {"bar": 16}, "value": 250},
            {"position": {"bar": 20}, "value": 900},
            {"position": {"bar": 30}, "value": 100},
        ]}),
    )
    .result()
    .expect("automation.set_lane succeeds");
    let before_lanes = lanes(&mut app).lanes;
    let before = app.revision();

    let written = shape_ok(&mut app, cutoff_example());
    assert_eq!(written.revision, before + 1, "one call, one revision");
    assert_eq!(app.revision(), before + 1);
    let shaped = written.lane.expect("lane");
    assert_eq!(shaped.points.len(), 1 + 8 * 16 + 1 + 1);

    let _: serde_json::Value =
        crate::common::roundtrip(&mut app, Request::without_params(98, "edit.undo"))
            .result()
            .expect("edit.undo succeeds");
    assert_eq!(lanes(&mut app).lanes, before_lanes, "undo restores exactly");

    let _: serde_json::Value =
        crate::common::roundtrip(&mut app, Request::without_params(97, "edit.redo"))
            .result()
            .expect("edit.redo succeeds");
    assert_eq!(lanes(&mut app).lanes, vec![shaped]);
}

#[test]
fn a_new_lane_is_undone_to_no_lane() {
    let mut app = app();
    let _ = shape_ok(&mut app, cutoff_example());
    let _: serde_json::Value =
        crate::common::roundtrip(&mut app, Request::without_params(98, "edit.undo"))
            .result()
            .expect("edit.undo succeeds");
    assert!(app.test_automation().lanes.is_empty());
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[test]
fn too_many_points_names_the_largest_resolution_that_fits() {
    let mut app = app();
    let err = error_of(shape(
        &mut app,
        json!({"track_id": TRACK, "param": "Filter Cutoff", "start": {"bar": 1},
               "end": {"bar": 5}, "shape": "sine", "from": 300, "to": 4000,
               "resolution": 1000}),
    ));
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
    // 4 bars: (2048 - 1) / 4 = 511 per bar, plus the end point, fits.
    assert!(err.message.contains("resolution <= 511"), "{}", err.message);
    assert!(app.test_automation().lanes.is_empty());

    let fits = shape_ok(
        &mut app,
        json!({"track_id": TRACK, "param": "Filter Cutoff", "start": {"bar": 1},
               "end": {"bar": 5}, "shape": "sine", "from": 300, "to": 4000,
               "resolution": 511}),
    );
    assert_eq!(fits.lane.expect("lane").points.len(), 511 * 4 + 1);
}

#[test]
fn end_must_lie_after_start() {
    let mut app = app();
    for end in [json!({"bar": 17}), json!({"bar": 9})] {
        let err = error_of(shape(
            &mut app,
            json!({"track_id": TRACK, "control": "volume", "start": {"bar": 17},
                   "end": end, "shape": "ramp", "from": -6, "to": 0}),
        ));
        assert_eq!(err.kind(), ErrorKind::InvalidParams);
        assert!(err.message.contains("after start"), "{}", err.message);
    }
    assert!(app.test_automation().lanes.is_empty());
}

#[test]
fn out_of_range_and_non_positive_exp_values_are_refused() {
    let mut app = app();
    let err = error_of(shape(
        &mut app,
        json!({"track_id": TRACK, "control": "volume", "start": {"bar": 1},
               "end": {"bar": 2}, "shape": "ramp", "from": -6, "to": 12}),
    ));
    assert!(err.message.starts_with("to: "), "{}", err.message);
    assert!(err.message.contains("-60..=6"), "{}", err.message);

    let err = error_of(shape(
        &mut app,
        json!({"track_id": TRACK, "param": "Filter Cutoff", "start": {"bar": 1},
               "end": {"bar": 2}, "shape": "exp", "from": 300, "to": 30000}),
    ));
    assert!(err.message.starts_with("to: "), "{}", err.message);
    assert!(app.test_automation().lanes.is_empty());
}
