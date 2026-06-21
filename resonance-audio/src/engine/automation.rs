//! Engine-thread handlers for parameter-automation lanes (doc #162 §2,
//! epic #14).
//!
//! Lanes live in engine-thread-local state ([`HandlerState::automation_lanes`]),
//! keyed by [`AutomationTarget`] — one lane per target. Storing a lane
//! replaces any existing entry wholesale; clearing removes it; the
//! per-lane "read" flag toggles [`AutomationLane::enabled`] in place.
//! Every mutation echoes the resulting engine state back to the app via
//! an [`AudioEvent`] so the app mirror stays in lock-step.
//!
//! No audio is applied here — a later step samples these lanes per block.
//! Points are kept sorted on store so that future per-block evaluation
//! can binary-search ([`resonance_common::sample_lane`]) without sorting
//! or allocating on the audio thread.

use std::collections::HashMap;

use crossbeam_channel::Sender;
use resonance_common::{AutomationLane, AutomationTarget};

use crate::types::AudioEvent;

/// Engine-thread-local map of automation lanes, one per target.
pub type AutomationLanes = HashMap<AutomationTarget, AutomationLane>;

/// Store or replace the lane for its target, then echo the stored lane
/// back via `AutomationLaneChanged`. The breakpoints are re-sorted on the
/// way in so the invariant holds even if a lane was assembled without
/// going through [`AutomationLane::new`].
pub fn set_automation_lane_in_place(
    lanes: &mut AutomationLanes,
    event_tx: &Sender<AudioEvent>,
    mut lane: AutomationLane,
) {
    lane.sort_points();
    let target = lane.target;
    lanes.insert(target, lane.clone());
    let _ = event_tx.send(AudioEvent::AutomationLaneChanged { lane });
}

/// Remove the lane stored for `target`. Emits `AutomationLaneCleared`
/// only when a lane was actually present, mirroring the clip handlers'
/// "missing lookup ⇒ no event" convention.
pub fn clear_automation_lane_in_place(
    lanes: &mut AutomationLanes,
    event_tx: &Sender<AudioEvent>,
    target: AutomationTarget,
) {
    if lanes.remove(&target).is_some() {
        let _ = event_tx.send(AudioEvent::AutomationLaneCleared { target });
    }
}

/// Toggle the per-lane read flag (`enabled`) without touching the
/// breakpoints. Echoes the updated lane via `AutomationLaneChanged`.
/// No-op (no event) when no lane is stored for `target`.
pub fn set_automation_read_enabled_in_place(
    lanes: &mut AutomationLanes,
    event_tx: &Sender<AudioEvent>,
    target: AutomationTarget,
    enabled: bool,
) {
    if let Some(lane) = lanes.get_mut(&target) {
        lane.enabled = enabled;
        let lane = lane.clone();
        let _ = event_tx.send(AudioEvent::AutomationLaneChanged { lane });
    }
}
