//! Tests for the throttled live automated-value emitter (todo #377).
//!
//! Drives [`LiveValueEmitter::poll`] / `reset` directly — the pure,
//! allocation-free core the engine control loop wraps with a ~30 ms
//! wall-clock throttle and an `AudioEvent::AutomatedValue` send per
//! result. Keeping the test on the pure helper means no engine thread,
//! no transport, and no wall-clock timing: the `frame` argument stands in
//! for the playhead position the loop would read each tick.

use resonance_audio::{AutomationLanes, LiveValueEmitter, AUTOMATED_VALUE_EPSILON};
use resonance_common::automation::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};

fn lin(time: u64, value: f32) -> Breakpoint {
    Breakpoint::new(time, value, CurveKind::Linear)
}

/// A linear lane for the given target rising 0.0 → 1.0 over 0..=1000 frames.
fn ramp_lane(id: u64, target: AutomationTarget) -> AutomationLane {
    AutomationLane::new(id, target, vec![lin(0, 0.0), lin(1000, 1.0)])
}

fn lanes_with(lane: AutomationLane) -> AutomationLanes {
    let mut lanes = AutomationLanes::new();
    lanes.insert(lane.target.clone(), lane);
    lanes
}

#[test]
fn first_poll_emits_each_enabled_lane() {
    let target = AutomationTarget::TrackGain(1);
    let lanes = lanes_with(ramp_lane(1, target.clone()));
    let mut emitter = LiveValueEmitter::default();

    let batch = emitter.poll(&lanes, 0);
    assert_eq!(
        batch,
        &[(target, 0.0)],
        "the first poll emits the lane value at the playhead"
    );
}

#[test]
fn moving_playhead_emits_changed_value_only() {
    let target = AutomationTarget::TrackGain(1);
    let lanes = lanes_with(ramp_lane(1, target.clone()));
    let mut emitter = LiveValueEmitter::default();

    // Prime at frame 500 (value 0.5).
    let first = emitter.poll(&lanes, 500).to_vec();
    assert_eq!(first, vec![(target.clone(), 0.5)]);

    // Re-poll at the SAME frame: value unchanged ⇒ nothing emitted.
    assert!(
        emitter.poll(&lanes, 500).is_empty(),
        "an unchanged value is suppressed so a flat region doesn't spam events"
    );

    // Advance the playhead: the new value is past the epsilon ⇒ emitted.
    let moved = emitter.poll(&lanes, 750).to_vec();
    assert_eq!(moved, vec![(target, 0.75)]);
}

#[test]
fn sub_epsilon_move_is_suppressed() {
    let target = AutomationTarget::TrackGain(1);
    let lanes = lanes_with(ramp_lane(1, target.clone()));
    let mut emitter = LiveValueEmitter::default();

    let _ = emitter.poll(&lanes, 500); // prime at 0.5

    // A frame step that moves the value by less than one epsilon must not
    // emit. The ramp covers 1.0 over 1000 frames, so frames map 1:1000 to
    // value; pick a step strictly under EPSILON * 1000 frames.
    let tiny_step = ((AUTOMATED_VALUE_EPSILON * 1000.0) as u64).saturating_sub(1);
    assert!(
        emitter.poll(&lanes, 500 + tiny_step).is_empty(),
        "a value move below the epsilon is suppressed"
    );
}

#[test]
fn read_disabled_lane_is_never_emitted() {
    let target = AutomationTarget::TrackGain(1);
    let mut lane = ramp_lane(1, target);
    lane.enabled = false; // Read off
    let lanes = lanes_with(lane);
    let mut emitter = LiveValueEmitter::default();

    assert!(
        emitter.poll(&lanes, 500).is_empty(),
        "a Read-off lane uses the static value and emits no live updates"
    );
    assert!(emitter.is_idle(), "nothing was memoized for a disabled lane");
}

#[test]
fn distinct_targets_tracked_independently() {
    let gain = AutomationTarget::TrackGain(1);
    let pan = AutomationTarget::TrackPan(2);
    let mut lanes = AutomationLanes::new();
    lanes.insert(gain.clone(), ramp_lane(1, gain.clone()));
    lanes.insert(pan.clone(), ramp_lane(2, pan.clone()));
    let mut emitter = LiveValueEmitter::default();

    let batch = emitter.poll(&lanes, 250).to_vec();
    assert_eq!(batch.len(), 2, "both enabled lanes emit on the first poll");
    assert!(batch.contains(&(gain, 0.25)), "gain lane value emitted");
    assert!(batch.contains(&(pan, 0.25)), "pan lane value emitted");
}

#[test]
fn reset_re_emits_unchanged_value() {
    let target = AutomationTarget::TrackGain(1);
    let lanes = lanes_with(ramp_lane(1, target.clone()));
    let mut emitter = LiveValueEmitter::default();

    let _ = emitter.poll(&lanes, 500); // prime at 0.5
    assert!(!emitter.is_idle());
    assert!(emitter.poll(&lanes, 500).is_empty(), "second poll suppressed");

    // Transport stop clears the memo; replay from the same frame re-tints.
    emitter.reset();
    assert!(emitter.is_idle());
    assert_eq!(
        emitter.poll(&lanes, 500).to_vec(),
        vec![(target, 0.5)],
        "after reset the same frame emits a fresh value again"
    );
}
