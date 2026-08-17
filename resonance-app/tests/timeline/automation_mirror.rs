//! Engine→app mirroring of parameter-automation lanes (todo #378, arch
//! doc #162 §3). The engine owns the authoritative lanes and live values;
//! these tests prove that receiving `AudioEvent::AutomationLaneChanged` /
//! `AutomationLaneCleared` / `AutomatedValue` reconstructs the GUI-side
//! `AutomationState` keyed by target, and that clearing a lane drops its
//! live value so a stale fader/knob tint can't outlive the lane.

use resonance_app::Resonance;
use resonance_audio::types::AudioEvent;
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};

/// A two-point linear lane for `target`.
fn lane(id: u64, target: AutomationTarget) -> AutomationLane {
    AutomationLane::new(
        id,
        target,
        vec![
            Breakpoint::new(0, 0.0, CurveKind::Linear),
            Breakpoint::new(48_000, 1.0, CurveKind::Linear),
        ],
    )
}

#[test]
fn lane_changed_inserts_keyed_by_target() {
    let mut app = Resonance::new_for_test().0;
    let target = AutomationTarget::TrackGain(1);

    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged {
        lane: lane(10, target.clone()),
    });

    let stored = app.test_automation().lanes.get(&target).unwrap();
    assert_eq!(stored.id, 10);
    assert_eq!(stored.points.len(), 2);
    assert!(stored.enabled);
}

#[test]
fn lane_changed_replaces_whole_lane_for_same_target() {
    let mut app = Resonance::new_for_test().0;
    let target = AutomationTarget::TrackPan(2);

    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged {
        lane: lane(10, target.clone()),
    });
    // A later store for the same target replaces, not appends — there is
    // one lane per target.
    let mut replacement = lane(10, target.clone());
    replacement.enabled = false;
    replacement.points.truncate(1);
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane: replacement });

    assert_eq!(app.test_automation().lanes.len(), 1);
    let stored = app.test_automation().lanes.get(&target).unwrap();
    assert!(!stored.enabled);
    assert_eq!(stored.points.len(), 1);
}

#[test]
fn distinct_targets_coexist() {
    let mut app = Resonance::new_for_test().0;
    let gain = AutomationTarget::TrackGain(1);
    let pan = AutomationTarget::TrackPan(1);

    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane: lane(10, gain.clone()) });
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane: lane(11, pan.clone()) });

    assert_eq!(app.test_automation().lanes.len(), 2);
    assert!(app.test_automation().lanes.contains_key(&gain));
    assert!(app.test_automation().lanes.contains_key(&pan));
}

#[test]
fn lane_cleared_removes_lane_and_live_value() {
    let mut app = Resonance::new_for_test().0;
    let target = AutomationTarget::MasterGain;

    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane: lane(10, target.clone()) });
    app.test_apply_engine_event(AudioEvent::AutomatedValue {
        target: target.clone(),
        value_norm: 0.75,
    });
    assert!(app.test_automation().lanes.contains_key(&target));
    assert_eq!(app.test_automation().live_values.get(&target), Some(&0.75));

    app.test_apply_engine_event(AudioEvent::AutomationLaneCleared { target: target.clone() });

    // Both the lane and its transient live value are gone.
    assert!(!app.test_automation().lanes.contains_key(&target));
    assert!(!app.test_automation().live_values.contains_key(&target));
}

#[test]
fn automated_value_tracks_latest_per_target() {
    let mut app = Resonance::new_for_test().0;
    let target = AutomationTarget::PluginParam {
        instance: 5,
        param_id: 3,
    };

    app.test_apply_engine_event(AudioEvent::AutomatedValue {
        target: target.clone(),
        value_norm: 0.2,
    });
    app.test_apply_engine_event(AudioEvent::AutomatedValue {
        target: target.clone(),
        value_norm: 0.6,
    });

    // Latest value wins; live values are independent of lane presence.
    assert_eq!(app.test_automation().live_values.get(&target), Some(&0.6));
    assert!(app.test_automation().lanes.is_empty());
}

#[test]
fn clearing_unknown_target_is_a_no_op() {
    let mut app = Resonance::new_for_test().0;
    let present = AutomationTarget::TrackGain(1);
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane: lane(10, present.clone()) });

    // No lane for this target — must not panic or disturb the present one.
    app.test_apply_engine_event(AudioEvent::AutomationLaneCleared {
        target: AutomationTarget::TrackGain(99),
    });

    assert_eq!(app.test_automation().lanes.len(), 1);
    assert!(app.test_automation().lanes.contains_key(&present));
}
