//! Tests for the automation-lane command boundary (todo #375).
//!
//! Drives the engine-internal pure helpers `set_automation_lane_in_place`,
//! `clear_automation_lane_in_place` and `set_automation_read_enabled_in_place`
//! directly via the `#[doc(hidden)]` re-exports. That keeps the test
//! headless — no cpal stream, no engine thread, no audio device — while
//! exercising the exact store/replace/clear + read-flag toggle + event
//! emission the `AudioCommand::SetAutomationLane` /
//! `ClearAutomationLane` / `SetAutomationReadEnabled` dispatch path runs.

use crossbeam_channel::unbounded;

use resonance_audio::types::AudioEvent;
use resonance_audio::{
    clear_automation_lane_in_place, set_automation_lane_in_place,
    set_automation_read_enabled_in_place, AutomationLanes,
};
use resonance_common::automation::{
    AutomationLane, AutomationTarget, Breakpoint, CurveKind,
};

fn lin(time: u64, value: f32) -> Breakpoint {
    Breakpoint::new(time, value, CurveKind::Linear)
}

/// A gain lane for track 1 with the given (unsorted-tolerant) points.
fn gain_lane(id: u64, points: Vec<Breakpoint>) -> AutomationLane {
    AutomationLane::new(id, AutomationTarget::TrackGain(1), points)
}

#[test]
fn set_lane_stores_and_emits_event() {
    let mut lanes = AutomationLanes::new();
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    let lane = gain_lane(42, vec![lin(0, 0.0), lin(480, 1.0)]);
    set_automation_lane_in_place(&mut lanes, &event_tx, lane.clone());

    match event_rx.try_recv() {
        Ok(AudioEvent::AutomationLaneChanged { lane: echoed }) => {
            assert_eq!(echoed, lane, "echoed lane mirrors stored lane");
        }
        other => panic!("expected AutomationLaneChanged, got {other:?}"),
    }
    assert!(
        event_rx.try_recv().is_err(),
        "exactly one event should be emitted"
    );

    let stored = lanes
        .get(&AutomationTarget::TrackGain(1))
        .expect("lane stored under its target");
    assert_eq!(*stored, lane);
}

#[test]
fn set_lane_sorts_points_on_store() {
    let mut lanes = AutomationLanes::new();
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    // Hand-build a lane with out-of-order points, bypassing the sorting
    // `AutomationLane::new` so the handler's own sort is what we observe.
    let lane = AutomationLane {
        id: 1,
        target: AutomationTarget::TrackGain(1),
        enabled: true,
        points: vec![lin(480, 1.0), lin(0, 0.0), lin(240, 0.5)],
    };
    set_automation_lane_in_place(&mut lanes, &event_tx, lane);

    let times: Vec<u64> = lanes[&AutomationTarget::TrackGain(1)]
        .points
        .iter()
        .map(|p| p.time_frames)
        .collect();
    assert_eq!(times, vec![0, 240, 480], "points stored sorted by time");

    match event_rx.try_recv() {
        Ok(AudioEvent::AutomationLaneChanged { lane }) => {
            let echoed: Vec<u64> = lane.points.iter().map(|p| p.time_frames).collect();
            assert_eq!(echoed, vec![0, 240, 480], "echoed lane is sorted too");
        }
        other => panic!("expected AutomationLaneChanged, got {other:?}"),
    }
}

#[test]
fn set_lane_replaces_existing_for_same_target() {
    let mut lanes = AutomationLanes::new();
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    set_automation_lane_in_place(&mut lanes, &event_tx, gain_lane(1, vec![lin(0, 0.0)]));
    set_automation_lane_in_place(
        &mut lanes,
        &event_tx,
        gain_lane(2, vec![lin(0, 1.0), lin(960, 0.25)]),
    );

    // One target, the later lane wins wholesale.
    assert_eq!(lanes.len(), 1, "same target keeps a single entry");
    let stored = &lanes[&AutomationTarget::TrackGain(1)];
    assert_eq!(stored.id, 2);
    assert_eq!(stored.points.len(), 2);

    // Two store events, one per call.
    assert!(matches!(
        event_rx.try_recv(),
        Ok(AudioEvent::AutomationLaneChanged { .. })
    ));
    assert!(matches!(
        event_rx.try_recv(),
        Ok(AudioEvent::AutomationLaneChanged { .. })
    ));
    assert!(event_rx.try_recv().is_err());
}

#[test]
fn distinct_targets_coexist() {
    let mut lanes = AutomationLanes::new();
    let (event_tx, _event_rx) = unbounded::<AudioEvent>();

    set_automation_lane_in_place(&mut lanes, &event_tx, gain_lane(1, vec![lin(0, 0.0)]));
    set_automation_lane_in_place(
        &mut lanes,
        &event_tx,
        AutomationLane::new(2, AutomationTarget::TrackPan(1), vec![lin(0, 0.5)]),
    );
    set_automation_lane_in_place(
        &mut lanes,
        &event_tx,
        AutomationLane::new(3, AutomationTarget::MasterGain, vec![lin(0, 0.8)]),
    );

    assert_eq!(lanes.len(), 3, "one entry per distinct target");
    assert!(lanes.contains_key(&AutomationTarget::TrackGain(1)));
    assert!(lanes.contains_key(&AutomationTarget::TrackPan(1)));
    assert!(lanes.contains_key(&AutomationTarget::MasterGain));
}

#[test]
fn clear_lane_removes_and_emits_event() {
    let mut lanes = AutomationLanes::new();
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    set_automation_lane_in_place(&mut lanes, &event_tx, gain_lane(1, vec![lin(0, 0.0)]));
    let _ = event_rx.try_recv(); // drain the store event

    clear_automation_lane_in_place(&mut lanes, &event_tx, AutomationTarget::TrackGain(1));

    match event_rx.try_recv() {
        Ok(AudioEvent::AutomationLaneCleared { target }) => {
            assert_eq!(target, AutomationTarget::TrackGain(1));
        }
        other => panic!("expected AutomationLaneCleared, got {other:?}"),
    }
    assert!(event_rx.try_recv().is_err(), "exactly one event");
    assert!(lanes.is_empty(), "lane removed from engine state");
}

#[test]
fn clear_missing_target_emits_no_event() {
    let mut lanes = AutomationLanes::new();
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    clear_automation_lane_in_place(&mut lanes, &event_tx, AutomationTarget::MasterGain);

    assert!(
        event_rx.try_recv().is_err(),
        "AutomationLaneCleared must not be emitted for an absent target"
    );
}

#[test]
fn set_read_enabled_toggles_flag_and_emits_event() {
    let mut lanes = AutomationLanes::new();
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    // Lanes start enabled (read on) via `AutomationLane::new`.
    set_automation_lane_in_place(&mut lanes, &event_tx, gain_lane(1, vec![lin(0, 0.0)]));
    let _ = event_rx.try_recv(); // drain the store event

    set_automation_read_enabled_in_place(
        &mut lanes,
        &event_tx,
        AutomationTarget::TrackGain(1),
        false,
    );

    match event_rx.try_recv() {
        Ok(AudioEvent::AutomationLaneChanged { lane }) => {
            assert!(!lane.enabled, "echoed lane reflects read-off");
            assert_eq!(lane.id, 1, "breakpoints/identity untouched");
        }
        other => panic!("expected AutomationLaneChanged, got {other:?}"),
    }
    assert!(
        !lanes[&AutomationTarget::TrackGain(1)].enabled,
        "read flag toggled in engine state"
    );

    // Toggle back on.
    set_automation_read_enabled_in_place(
        &mut lanes,
        &event_tx,
        AutomationTarget::TrackGain(1),
        true,
    );
    assert!(lanes[&AutomationTarget::TrackGain(1)].enabled);
}

#[test]
fn set_read_enabled_missing_target_emits_no_event() {
    let mut lanes = AutomationLanes::new();
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    set_automation_read_enabled_in_place(
        &mut lanes,
        &event_tx,
        AutomationTarget::TrackGain(99),
        false,
    );

    assert!(
        event_rx.try_recv().is_err(),
        "no event when toggling an absent target"
    );
    assert!(lanes.is_empty());
}
