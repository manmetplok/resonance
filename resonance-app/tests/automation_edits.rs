//! Update-layer coverage for parameter-automation lane edits (todo #380,
//! arch doc #162 §3). These drive the real reducer through
//! `Resonance::update` and assert the resulting app-side `AutomationState`
//! mirror — proving each edit message mutates the right lane optimistically
//! (the engine echo would re-apply the same change idempotently). Also pins
//! the undo classification and an undo/redo round-trip for an automation
//! edit, which rides through the structure-preserving diff replay.

use resonance_app::message::{AutomationMessage, Message};
use resonance_app::state::TrackState;
use resonance_app::Resonance;
use resonance_common::{
    real_to_lane_value, AutomationTarget, CurveKind, GAIN_MAX_DB, GAIN_MIN_DB,
};

/// One audio track at a known volume so seeding has something to read.
fn app_with_track(id: u64, volume_db: f32) -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    let mut track = TrackState::new_audio(id, 0);
    track.volume = volume_db;
    app.test_push_track(track);
    app
}

fn send(app: &mut Resonance, m: AutomationMessage) {
    let _ = app.update(Message::Automation(m));
}

#[test]
fn add_lane_seeds_one_breakpoint_at_current_value() {
    // -6 dB sits midway-ish in the -60..+6 dB fader range.
    let mut app = app_with_track(1, -6.0);
    let target = AutomationTarget::TrackGain(1);

    send(&mut app, AutomationMessage::AddLane(target));

    let lane = app.test_automation().lanes.get(&target).expect("lane added");
    assert!(lane.enabled, "a freshly-added lane reads by default");
    assert_eq!(lane.points.len(), 1, "seeded with exactly one breakpoint");
    assert_eq!(lane.points[0].time_frames, 0);
    let expected = real_to_lane_value(target, -6.0);
    assert!((lane.points[0].value - expected).abs() < 1e-6);
    // The seeded value round-trips back to the static dB it came from.
    let real = GAIN_MIN_DB + lane.points[0].value * (GAIN_MAX_DB - GAIN_MIN_DB);
    assert!((real - (-6.0)).abs() < 1e-3);
}

#[test]
fn add_lane_is_noop_when_lane_exists() {
    let mut app = app_with_track(1, 0.0);
    let target = AutomationTarget::TrackGain(1);
    send(&mut app, AutomationMessage::AddLane(target));
    let id = app.test_automation().lanes[&target].id;

    // A second add must not replace or duplicate the lane.
    send(&mut app, AutomationMessage::AddLane(target));
    assert_eq!(app.test_automation().lanes.len(), 1);
    assert_eq!(app.test_automation().lanes[&target].id, id);
}

#[test]
fn remove_lane_drops_lane_and_live_value() {
    let mut app = app_with_track(1, 0.0);
    let target = AutomationTarget::TrackPan(1);
    send(&mut app, AutomationMessage::AddLane(target));
    assert!(app.test_automation().lanes.contains_key(&target));

    send(&mut app, AutomationMessage::RemoveLane(target));
    assert!(!app.test_automation().lanes.contains_key(&target));
}

#[test]
fn toggle_read_flips_enabled() {
    let mut app = app_with_track(1, 0.0);
    let target = AutomationTarget::TrackGain(1);
    send(&mut app, AutomationMessage::AddLane(target));
    assert!(app.test_automation().lanes[&target].enabled);

    send(&mut app, AutomationMessage::ToggleRead(target));
    assert!(!app.test_automation().lanes[&target].enabled);
    send(&mut app, AutomationMessage::ToggleRead(target));
    assert!(app.test_automation().lanes[&target].enabled);
}

#[test]
fn add_breakpoint_creates_lane_and_keeps_points_sorted() {
    let mut app = app_with_track(1, 0.0);
    let target = AutomationTarget::TrackGain(1);

    // Add out of time order; the lane must end up sorted.
    send(
        &mut app,
        AutomationMessage::AddBreakpoint {
            target,
            time_frames: 48_000,
            value: 1.0,
            curve: CurveKind::Linear,
        },
    );
    send(
        &mut app,
        AutomationMessage::AddBreakpoint {
            target,
            time_frames: 0,
            value: 0.0,
            curve: CurveKind::Linear,
        },
    );

    let lane = app.test_automation().lanes.get(&target).expect("lane");
    let times: Vec<u64> = lane.points.iter().map(|p| p.time_frames).collect();
    assert_eq!(times, vec![0, 48_000]);
}

#[test]
fn delete_last_breakpoint_clears_the_lane() {
    let mut app = app_with_track(1, 0.0);
    let target = AutomationTarget::TrackGain(1);
    // Seeded lane has exactly one breakpoint.
    send(&mut app, AutomationMessage::AddLane(target));

    send(
        &mut app,
        AutomationMessage::DeleteBreakpoint { target, index: 0 },
    );
    // An enabled empty lane would silence the target, so it's cleared.
    assert!(!app.test_automation().lanes.contains_key(&target));
}

#[test]
fn delete_breakpoint_keeps_remaining_points() {
    let mut app = app_with_track(1, 0.0);
    let target = AutomationTarget::TrackGain(1);
    for (t, v) in [(0u64, 0.0f32), (24_000, 0.5), (48_000, 1.0)] {
        send(
            &mut app,
            AutomationMessage::AddBreakpoint {
                target,
                time_frames: t,
                value: v,
                curve: CurveKind::Linear,
            },
        );
    }

    send(
        &mut app,
        AutomationMessage::DeleteBreakpoint { target, index: 1 },
    );
    let lane = app.test_automation().lanes.get(&target).expect("lane");
    let times: Vec<u64> = lane.points.iter().map(|p| p.time_frames).collect();
    assert_eq!(times, vec![0, 48_000]);
}

#[test]
fn set_curve_kind_updates_the_point() {
    let mut app = app_with_track(1, 0.0);
    let target = AutomationTarget::TrackMute(1);
    send(
        &mut app,
        AutomationMessage::AddBreakpoint {
            target,
            time_frames: 0,
            value: 1.0,
            curve: CurveKind::Linear,
        },
    );

    send(
        &mut app,
        AutomationMessage::SetCurveKind {
            target,
            index: 0,
            curve: CurveKind::Stepped,
        },
    );
    assert_eq!(
        app.test_automation().lanes[&target].points[0].curve,
        CurveKind::Stepped
    );
}

#[test]
fn drag_breakpoint_moves_and_resorts() {
    let mut app = app_with_track(1, 0.0);
    let target = AutomationTarget::TrackGain(1);
    for (t, v) in [(0u64, 0.0f32), (48_000, 1.0)] {
        send(
            &mut app,
            AutomationMessage::AddBreakpoint {
                target,
                time_frames: t,
                value: v,
                curve: CurveKind::Linear,
            },
        );
    }

    // Drag the first point past the second in time: the lane re-sorts.
    send(
        &mut app,
        AutomationMessage::DragBreakpoint {
            target,
            index: 0,
            time_frames: 96_000,
            value: 0.25,
        },
    );
    let lane = app.test_automation().lanes.get(&target).expect("lane");
    let times: Vec<u64> = lane.points.iter().map(|p| p.time_frames).collect();
    assert_eq!(times, vec![48_000, 96_000]);
    // The moved point kept its new value at its new (last) position.
    assert!((lane.points[1].value - 0.25).abs() < 1e-6);
}

#[test]
fn classify_undo_actions() {
    use resonance_app::undo::{classify, UndoAction};
    let target = AutomationTarget::TrackGain(1);

    let is_record = |m: AutomationMessage| {
        matches!(classify(&Message::Automation(m)), UndoAction::Record)
    };
    assert!(is_record(AutomationMessage::AddLane(target)));
    assert!(is_record(AutomationMessage::RemoveLane(target)));
    assert!(is_record(AutomationMessage::ToggleRead(target)));
    assert!(is_record(AutomationMessage::DeleteBreakpoint { target, index: 0 }));

    assert!(matches!(
        classify(&Message::Automation(AutomationMessage::StartBreakpointDrag {
            target,
            index: 0
        })),
        UndoAction::Begin
    ));
    assert!(matches!(
        classify(&Message::Automation(AutomationMessage::DragBreakpoint {
            target,
            index: 0,
            time_frames: 0,
            value: 0.0
        })),
        UndoAction::Skip
    ));
    assert!(matches!(
        classify(&Message::Automation(AutomationMessage::EndBreakpointDrag)),
        UndoAction::Commit
    ));
}

#[test]
fn undo_redo_round_trips_an_added_lane() {
    let mut app = app_with_track(1, 0.0);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/resonance-test"));
    let target = AutomationTarget::TrackGain(1);

    send(&mut app, AutomationMessage::AddLane(target));
    assert!(app.test_automation().lanes.contains_key(&target));

    // Undo removes the lane (structure-preserving diff replay reconciles
    // the engine + mirror back to the pre-edit empty set).
    let _ = app.update(Message::Undo);
    assert!(
        !app.test_automation().lanes.contains_key(&target),
        "undo should drop the added lane"
    );

    // Redo brings it back.
    let _ = app.update(Message::Redo);
    assert!(
        app.test_automation().lanes.contains_key(&target),
        "redo should restore the lane"
    );
}

#[test]
fn edits_are_gated_without_an_active_project() {
    let (mut app, _task) = Resonance::new();
    // No active project: the startup-modal gate must swallow automation
    // edits just like every other project-mutating message.
    send(&mut app, AutomationMessage::AddLane(AutomationTarget::MasterGain));
    assert!(app.test_automation().lanes.is_empty());
}
