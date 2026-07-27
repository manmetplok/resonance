//! App-side handlers for parameter-automation events from the engine
//! (architecture doc #162 §3, todo #378).
//!
//! One-way engine→app mirror: the engine owns the authoritative lanes and
//! emits `AutomationLaneChanged` / `AutomationLaneCleared` so the app can
//! reconstruct its [`crate::state::AutomationState`] (e.g. after a project
//! load replays `SetAutomationLane`). `AutomatedValue` carries the
//! throttled live value during playback into the transient live-value map.

use resonance_common::{AutomationLane, AutomationTarget};

use crate::Resonance;

/// A lane was stored, replaced, or had its read flag toggled. Mirror it
/// into app state keyed by target — one lane per target, whole-lane
/// replace, matching the engine's storage.
pub(super) fn lane_changed(r: &mut Resonance, lane: AutomationLane) {
    r.automation.lanes.insert(lane.target.clone(), lane);
}

/// The lane for `target` was removed from engine state. Drop the app
/// mirror and any live value so a stale fader/knob tint can't outlive the
/// lane.
pub(super) fn lane_cleared(r: &mut Resonance, target: AutomationTarget) {
    r.automation.lanes.remove(&target);
    r.automation.live_values.remove(&target);
}

/// Record the latest throttled automated value for `target` in the
/// transient live-value map (consumed by the fader/knob tint while Read is
/// on — todo #383). The engine-side throttled emission lands in todo #377;
/// until then this arm simply never fires.
pub(super) fn automated_value(r: &mut Resonance, target: AutomationTarget, value_norm: f32) {
    r.automation.live_values.insert(target, value_norm);
}
