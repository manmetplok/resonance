//! `automation.add_points` / `automation.delete_points` /
//! `automation.set_enabled` / `automation.remove_lane`
//! (automation-control-api.md slices A4 + A5).
//!
//! `automation.lanes` / `automation.set_lane` (A2 + A3) already cover the
//! shared target resolution, value mapping and tempo/meter behaviour
//! (`control_automation.rs`); this module covers what is new here: the
//! upsert-on-occupied-frame contract (D4), delete by index / by range,
//! the empty-lane confirm gate that both `delete_points` and
//! `remove_lane` share, `set_enabled`'s declarative no-bump ack, the
//! `lane_id` addressing that reaches an orphaned lane, and undo/redo +
//! the one-call-one-revision contract for each of the four methods.

use crate::common::call;
use resonance_app::message::{AutomationMessage, Message};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_common::{AutomationTarget, CurveKind};
use resonance_control::methods::automation::{AutomationValue, LaneEditResult, LanesResult};
use resonance_control::{ErrorKind, Request, Response};
use serde_json::json;

const TRACK: u64 = 1;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from(
        "/tmp/control-automation-edit.rprj",
    ));
    app.test_set_sample_rate(48_000);
    app.test_set_flat_tempo(120.0);
    app.test_add_track(TRACK, resonance_audio::types::TrackType::Instrument);
    app
}

fn volume_target() -> serde_json::Value {
    json!({"track_id": TRACK, "control": "volume"})
}

fn add_points(app: &mut Resonance, params: serde_json::Value) -> Response {
    call(app, "automation.add_points", params)
}

fn add_points_ok(app: &mut Resonance, params: serde_json::Value) -> LaneEditResult {
    add_points(app, params)
        .result()
        .expect("automation.add_points succeeds")
}

fn delete_points(app: &mut Resonance, params: serde_json::Value) -> Response {
    call(app, "automation.delete_points", params)
}

fn delete_points_ok(app: &mut Resonance, params: serde_json::Value) -> LaneEditResult {
    delete_points(app, params)
        .result()
        .expect("automation.delete_points succeeds")
}

fn set_enabled(app: &mut Resonance, params: serde_json::Value) -> Response {
    call(app, "automation.set_enabled", params)
}

fn set_enabled_ok(app: &mut Resonance, params: serde_json::Value) -> LaneEditResult {
    set_enabled(app, params)
        .result()
        .expect("automation.set_enabled succeeds")
}

fn remove_lane(app: &mut Resonance, params: serde_json::Value) -> Response {
    call(app, "automation.remove_lane", params)
}

fn remove_lane_ok(app: &mut Resonance, params: serde_json::Value) -> LaneEditResult {
    remove_lane(app, params)
        .result()
        .expect("automation.remove_lane succeeds")
}

fn set_lane_ok(app: &mut Resonance, params: serde_json::Value) -> LaneEditResult {
    call(app, "automation.set_lane", params)
        .result()
        .expect("automation.set_lane succeeds")
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

// ---------------------------------------------------------------------------
// add_points
// ---------------------------------------------------------------------------

#[test]
fn add_points_creates_a_lane_and_upserts_an_occupied_frame() {
    let mut app = app();
    let before = app.revision();

    let mut target = volume_target();
    target["points"] = json!([
        {"position": {"bar": 1}, "value": -6},
        {"position": {"bar": 5}, "value": 0},
    ]);
    let first = add_points_ok(&mut app, target);
    assert_eq!(first.revision, before + 1);
    assert_eq!(first.replaced, Some(0));
    let lane = first.lane.expect("lane created");
    assert_eq!(lane.point_count, 2);

    // bar 1 is occupied: this upserts it in place and adds bar 9.
    let mut again = volume_target();
    again["points"] = json!([
        {"position": {"bar": 1}, "value": -3},
        {"position": {"bar": 9}, "value": 6},
    ]);
    let second = add_points_ok(&mut app, again);
    assert_eq!(second.revision, first.revision + 1);
    assert_eq!(second.replaced, Some(1), "bar 1 was already occupied");
    let lane2 = second.lane.expect("lane");
    assert_eq!(lane2.point_count, 3, "bar 1 replaced in place, bar 9 added");
    let bar1 = lane2
        .points
        .iter()
        .find(|p| p.position.bar == 1)
        .expect("bar 1 kept");
    assert_eq!(bar1.value, AutomationValue::Number(-3.0));
    assert!(lane2.points.iter().any(|p| p.position.bar == 5));
    assert!(lane2.points.iter().any(|p| p.position.bar == 9));
}

#[test]
fn add_points_rejects_two_inputs_on_the_same_frame() {
    let mut app = app();
    let mut target = volume_target();
    target["points"] = json!([
        {"position": {"bar": 1}, "value": -6},
        {"position": {"bar": 1}, "value": 0},
    ]);
    let err = error_of(add_points(&mut app, target));
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
    assert!(
        err.message.contains("points[0]") && err.message.contains("points[1]"),
        "{}",
        err.message
    );
    assert!(
        lanes(&mut app).lanes.is_empty(),
        "the refused call created nothing"
    );
}

// ---------------------------------------------------------------------------
// delete_points
// ---------------------------------------------------------------------------

fn seed_four_points(app: &mut Resonance) {
    let mut target = volume_target();
    target["points"] = json!([
        {"position": {"bar": 1}, "value": -6},
        {"position": {"bar": 2}, "value": -3},
        {"position": {"bar": 3}, "value": 0},
        {"position": {"bar": 4}, "value": 3},
    ]);
    set_lane_ok(app, target);
}

#[test]
fn delete_points_by_indices() {
    let mut app = app();
    seed_four_points(&mut app);
    let before = app.revision();

    let mut target = volume_target();
    target["indices"] = json!([1]);
    let result = delete_points_ok(&mut app, target);
    assert_eq!(result.revision, before + 1);
    assert_eq!(result.deleted, Some(1));
    let lane = result.lane.expect("lane survives");
    assert_eq!(lane.point_count, 3);
    assert!(!lane.points.iter().any(|p| p.position.bar == 2));
}

#[test]
fn delete_points_by_range() {
    let mut app = app();
    seed_four_points(&mut app);

    let mut target = volume_target();
    target["range"] = json!({"start": {"bar": 3}, "end": {"bar": 5}});
    let result = delete_points_ok(&mut app, target);
    assert_eq!(result.deleted, Some(2), "bars 3 and 4 fall in [3, 5)");
    let lane = result.lane.expect("lane survives");
    assert_eq!(lane.point_count, 2);
    assert!(lane.points.iter().all(|p| p.position.bar <= 2));
}

#[test]
fn delete_points_requires_exactly_one_of_indices_or_range() {
    let mut app = app();
    seed_four_points(&mut app);

    let neither = error_of(delete_points(&mut app, volume_target()));
    assert_eq!(neither.kind(), ErrorKind::InvalidParams);

    let mut both = volume_target();
    both["indices"] = json!([0]);
    both["range"] = json!({"start": {"bar": 1}, "end": {"bar": 2}});
    let both_err = error_of(delete_points(&mut app, both));
    assert_eq!(both_err.kind(), ErrorKind::InvalidParams);
}

#[test]
fn delete_points_out_of_range_index_names_the_point_count() {
    let mut app = app();
    seed_four_points(&mut app);

    let mut target = volume_target();
    target["indices"] = json!([9]);
    let err = error_of(delete_points(&mut app, target));
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
    assert!(err.message.contains('4'), "{}", err.message);
}

#[test]
fn delete_points_emptying_a_lane_without_confirm_is_refused_and_the_lane_is_intact() {
    let mut app = app();
    seed_four_points(&mut app);
    let before = app.revision();

    let mut target = volume_target();
    target["range"] = json!({"start": {"bar": 1}, "end": {"bar": 5}});
    let err = error_of(delete_points(&mut app, target));
    assert_eq!(err.kind(), ErrorKind::NeedsConfirmation);
    assert!(err.message.contains('4'), "{}", err.message);
    assert!(err.message.contains("bars"), "{}", err.message);

    assert_eq!(app.revision(), before, "the refusal costs no revision");
    let lane = lanes(&mut app)
        .lanes
        .into_iter()
        .next()
        .expect("lane intact");
    assert_eq!(lane.point_count, 4);
}

#[test]
fn delete_points_emptying_a_lane_with_confirm_removes_it() {
    let mut app = app();
    seed_four_points(&mut app);

    let mut target = volume_target();
    target["range"] = json!({"start": {"bar": 1}, "end": {"bar": 5}});
    target["confirm"] = json!(true);
    let result = delete_points_ok(&mut app, target);
    assert_eq!(result.deleted, Some(4));
    assert!(result.removed);
    assert!(result.lane.is_none());
    assert!(lanes(&mut app).lanes.is_empty());
}

// ---------------------------------------------------------------------------
// set_enabled
// ---------------------------------------------------------------------------

#[test]
fn set_enabled_is_a_no_op_ack_when_the_flag_already_matches() {
    let mut app = app();
    seed_four_points(&mut app); // starts enabled
    let before = app.revision();

    let mut target = volume_target();
    target["enabled"] = json!(true);
    let result = set_enabled_ok(&mut app, target);
    assert_eq!(result.revision, before, "no dispatch, no bump");
    assert_eq!(app.revision(), before);
    assert!(result.lane.expect("lane").enabled);
}

#[test]
fn set_enabled_bumps_once_when_the_flag_changes() {
    let mut app = app();
    seed_four_points(&mut app);
    let before = app.revision();

    let mut target = volume_target();
    target["enabled"] = json!(false);
    let result = set_enabled_ok(&mut app, target);
    assert_eq!(result.revision, before + 1);
    assert!(!result.lane.expect("lane").enabled);
}

#[test]
fn set_enabled_on_a_missing_lane_is_not_found() {
    let mut app = app();
    let mut target = volume_target();
    target["enabled"] = json!(false);
    let err = error_of(set_enabled(&mut app, target));
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

// ---------------------------------------------------------------------------
// remove_lane
// ---------------------------------------------------------------------------

#[test]
fn remove_lane_requires_confirm_when_the_lane_holds_points_and_leaves_it_intact() {
    let mut app = app();
    seed_four_points(&mut app);
    let before = app.revision();

    let err = error_of(remove_lane(&mut app, volume_target()));
    assert_eq!(err.kind(), ErrorKind::NeedsConfirmation);
    assert!(err.message.contains('4'), "{}", err.message);
    assert_eq!(app.revision(), before);
    assert_eq!(lanes(&mut app).lanes.len(), 1);
}

#[test]
fn remove_lane_with_confirm_removes_it() {
    let mut app = app();
    seed_four_points(&mut app);

    let mut target = volume_target();
    target["confirm"] = json!(true);
    let result = remove_lane_ok(&mut app, target);
    assert!(result.removed);
    assert!(result.lane.is_none());
    assert!(lanes(&mut app).lanes.is_empty());
}

#[test]
fn remove_lane_on_a_missing_lane_is_not_found() {
    let mut app = app();
    let err = error_of(remove_lane(&mut app, volume_target()));
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

// ---------------------------------------------------------------------------
// `lane_id` addressing (delete_points / remove_lane only): the sole way
// to reach an orphaned lane, which has no resolvable owner.
// ---------------------------------------------------------------------------

const ORPHAN: AutomationTarget = AutomationTarget::PluginParam {
    instance: 999,
    param_id: 3,
};

fn seed_orphan(app: &mut Resonance) -> u64 {
    let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
        target: ORPHAN,
        time_frames: 0,
        value: 0.25,
        curve: CurveKind::Linear,
    }));
    let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
        target: ORPHAN,
        time_frames: 96_000,
        value: 0.75,
        curve: CurveKind::Linear,
    }));
    app.test_automation().lanes[&ORPHAN].id
}

#[test]
fn lane_id_addresses_an_orphaned_lane_for_delete_points_and_remove_lane() {
    let mut app = app();
    let lane_id = seed_orphan(&mut app);

    // A target spec cannot reach it: no owner exists to resolve.
    assert!(lanes(&mut app).lanes.iter().any(|l| l.lane_id == lane_id
        && l.status == resonance_control::methods::automation::LaneStatus::Orphaned));

    let deleted = delete_points_ok(&mut app, json!({"lane_id": lane_id, "indices": [0]}));
    assert_eq!(deleted.deleted, Some(1));
    let lane = deleted.lane.expect("one point left");
    assert_eq!(lane.point_count, 1);

    let removed = remove_lane_ok(&mut app, json!({"lane_id": lane_id, "confirm": true}));
    assert!(removed.removed);
    assert!(app.test_automation().lanes.get(&ORPHAN).is_none());
}

#[test]
fn lane_id_and_a_target_together_is_refused() {
    let mut app = app();
    seed_four_points(&mut app);
    let lane_id = lanes(&mut app).lanes[0].lane_id;

    let mut both = volume_target();
    both["lane_id"] = json!(lane_id);
    let err = error_of(remove_lane(&mut app, both));
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
}

#[test]
fn neither_lane_id_nor_a_target_is_refused() {
    let mut app = app();
    let err = error_of(remove_lane(&mut app, json!({"confirm": true})));
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
}

// ---------------------------------------------------------------------------
// Undo / redo, one entry per call
// ---------------------------------------------------------------------------

#[test]
fn add_points_undo_redo() {
    let mut app = app();
    let mut target = volume_target();
    target["points"] = json!([{"position": {"bar": 1}, "value": -6}]);
    add_points_ok(&mut app, target);
    assert!(!lanes(&mut app).lanes.is_empty());

    undo(&mut app);
    assert!(lanes(&mut app).lanes.is_empty());
    redo(&mut app);
    assert_eq!(lanes(&mut app).lanes[0].point_count, 1);
}

#[test]
fn delete_points_undo_redo() {
    let mut app = app();
    seed_four_points(&mut app);

    let mut target = volume_target();
    target["indices"] = json!([0]);
    delete_points_ok(&mut app, target);
    assert_eq!(lanes(&mut app).lanes[0].point_count, 3);

    undo(&mut app);
    assert_eq!(lanes(&mut app).lanes[0].point_count, 4);
    redo(&mut app);
    assert_eq!(lanes(&mut app).lanes[0].point_count, 3);
}

#[test]
fn set_enabled_undo_redo() {
    let mut app = app();
    seed_four_points(&mut app);

    let mut target = volume_target();
    target["enabled"] = json!(false);
    set_enabled_ok(&mut app, target);
    assert!(!lanes(&mut app).lanes[0].enabled);

    undo(&mut app);
    assert!(lanes(&mut app).lanes[0].enabled);
    redo(&mut app);
    assert!(!lanes(&mut app).lanes[0].enabled);
}

#[test]
fn remove_lane_undo_redo() {
    let mut app = app();
    seed_four_points(&mut app);

    let mut target = volume_target();
    target["confirm"] = json!(true);
    remove_lane_ok(&mut app, target);
    assert!(lanes(&mut app).lanes.is_empty());

    undo(&mut app);
    assert_eq!(lanes(&mut app).lanes[0].point_count, 4);
    redo(&mut app);
    assert!(lanes(&mut app).lanes.is_empty());
}

// ---------------------------------------------------------------------------
// Capability
// ---------------------------------------------------------------------------

#[test]
fn hello_lists_the_new_methods() {
    let mut app = app();
    let hello: resonance_control::methods::control::HelloResult = call(
        &mut app,
        "control.hello",
        json!({"protocol_version": resonance_control::PROTOCOL_VERSION}),
    )
    .result()
    .expect("control.hello");
    for method in [
        "automation.add_points",
        "automation.delete_points",
        "automation.set_enabled",
        "automation.remove_lane",
    ] {
        assert!(
            hello.capabilities.iter().any(|c| c == method),
            "{method} missing from control.hello"
        );
    }
}
