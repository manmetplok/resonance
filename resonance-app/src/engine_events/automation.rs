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
    let target = lane.target.clone();
    r.automation.lanes.insert(lane.target.clone(), lane);
    if let AutomationTarget::PluginParam { instance, .. } = target {
        sync_preset_ignored_params(r, instance, true);
    }
}

/// Tell a plugin which of its params are automated (enabled lanes), so
/// its preset-modified comparison leaves them out (plugin-preset-library
/// D8, slice P5). `always` sends an empty list too — a lane change may
/// have removed the last one; a plugin just added only needs it when it
/// has lanes.
pub(crate) fn sync_preset_ignored_params(
    r: &mut Resonance,
    instance_id: resonance_audio::types::PluginInstanceId,
    always: bool,
) {
    let mut clap_ids: Vec<u32> = r
        .automation
        .lanes
        .values()
        .filter(|l| l.enabled)
        .filter_map(|l| match l.target {
            AutomationTarget::PluginParam { instance, param_id } if instance == instance_id => {
                Some(param_id)
            }
            _ => None,
        })
        .collect();
    if clap_ids.is_empty() && !always {
        return;
    }
    clap_ids.sort_unstable();
    let _ = r
        .engine
        .send(resonance_audio::AudioCommand::SetPluginPresetIgnoredParams {
            instance_id,
            clap_ids,
        });
}

/// The lane for `target` was removed from engine state. Drop the app
/// mirror and any live value so a stale fader/knob tint can't outlive the
/// lane.
pub(super) fn lane_cleared(r: &mut Resonance, target: AutomationTarget) {
    r.automation.lanes.remove(&target);
    r.automation.live_values.remove(&target);
    if let AutomationTarget::PluginParam { instance, .. } = target {
        sync_preset_ignored_params(r, instance, true);
    }
}

/// Record the latest throttled automated value for `target` in the
/// transient live-value map (consumed by the fader/knob tint while Read is
/// on — todo #383). The engine-side throttled emission lands in todo #377;
/// until then this arm simply never fires.
pub(super) fn automated_value(r: &mut Resonance, target: AutomationTarget, value_norm: f32) {
    r.automation.live_values.insert(target, value_norm);
}
