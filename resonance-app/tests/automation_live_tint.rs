//! Live automated-value tint feeding the mixer fader/knob (todo #383, arch
//! doc #162 §3). `AutomationState::live_value` is the single source the
//! strips read to decide whether — and with what value — to tint a
//! channel's fader / pan knob during playback. It must return the value
//! only while a Read-enabled lane is actively being driven, so a stale or
//! Read-disabled lane never paints a misleading tint.

use resonance_app::Resonance;
use resonance_audio::types::AudioEvent;
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};

fn flat_lane(id: u64, target: &AutomationTarget, enabled: bool) -> AutomationLane {
    let mut lane = AutomationLane::new(
        id,
        target.clone(),
        vec![Breakpoint::new(0, 0.5, CurveKind::Linear)],
    );
    lane.enabled = enabled;
    lane
}

#[test]
fn live_value_present_only_when_enabled_lane_is_driven() {
    let mut app = Resonance::new().0;
    let target = AutomationTarget::TrackGain(1);

    // A Read-enabled lane with a throttled value → tint shows that value.
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged {
        lane: flat_lane(10, &target, true),
    });
    app.test_apply_engine_event(AudioEvent::AutomatedValue {
        target: target.clone(),
        value_norm: 0.8,
    });
    assert_eq!(app.test_automation().live_value(target), Some(0.8));
}

#[test]
fn live_value_none_without_a_throttled_value() {
    let mut app = Resonance::new().0;
    let target = AutomationTarget::BusPan(2);

    // Lane exists and is enabled but no AutomatedValue has arrived yet
    // (e.g. transport stopped) — nothing to tint.
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged {
        lane: flat_lane(11, &target, true),
    });
    assert_eq!(app.test_automation().live_value(target), None);
}

#[test]
fn live_value_none_when_read_disabled() {
    let mut app = Resonance::new().0;
    let target = AutomationTarget::MasterGain;

    // A Read-disabled lane keeps its points but uses the static value, so
    // the fader must stay un-tinted even if a stale live value lingers.
    app.test_apply_engine_event(AudioEvent::AutomatedValue {
        target: target.clone(),
        value_norm: 0.9,
    });
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged {
        lane: flat_lane(12, &target, false),
    });
    assert_eq!(app.test_automation().live_value(target), None);
}

#[test]
fn live_value_none_for_untargeted_channel() {
    let app = Resonance::new().0;
    // No lanes at all — every channel reads None.
    assert_eq!(
        app.test_automation()
            .live_value(AutomationTarget::TrackGain(7)),
        None
    );
}
