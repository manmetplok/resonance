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
use std::time::Duration;

use crossbeam_channel::Sender;
use resonance_common::{AutomationLane, AutomationTarget, PluginInstanceId};

use crate::clap_host::PluginMap;
use crate::types::AudioEvent;

/// Engine-thread-local map of automation lanes, one per target.
pub type AutomationLanes = HashMap<AutomationTarget, AutomationLane>;

/// A plugin-parameter lane with the owning plugin's real `min..=max`
/// range resolved on the engine thread (where the FFI query and the
/// allocation are allowed). The audio thread then maps the lane's
/// normalized value into the plugin's range with
/// [`resonance_common::lane_value_to_plugin_param`] — no query, no
/// allocation — and queues it via `set_param`.
#[derive(Debug, Clone)]
pub struct ResolvedParamLane {
    pub param_id: u32,
    pub lane: AutomationLane,
    pub min: f64,
    pub max: f64,
}

/// Audio-thread-readable snapshot of every **enabled** automation lane,
/// published wait-free by the engine thread (via `ArcSwap`) whenever the
/// lane set changes. Read-disabled lanes are omitted entirely, so the
/// render core's rule is simply "present ⇒ automate, absent ⇒ use the
/// static engine value".
///
/// Splitting plugin-param lanes into their own per-instance map lets the
/// render core enumerate "every param lane for this plugin" while it
/// holds the plugin lock, without scanning the whole target set.
#[derive(Debug, Clone, Default)]
pub struct AutomationSnapshot {
    /// Gain / pan / mute lanes, keyed by target for O(1) per-track,
    /// per-bus and master lookup.
    pub mix_lanes: HashMap<AutomationTarget, AutomationLane>,
    /// Plugin-param lanes grouped by plugin instance, with each lane's
    /// real range already resolved.
    pub plugin_params: HashMap<PluginInstanceId, Vec<ResolvedParamLane>>,
}

impl AutomationSnapshot {
    /// Build a fresh snapshot from the engine-thread lane map. Only
    /// `enabled` lanes are included. For each plugin-param lane the
    /// owning plugin's `min..=max` is read once here (engine thread) so
    /// the audio thread never has to query it. A lane whose target
    /// plugin or param can't be found is dropped — it'll be picked up
    /// the next time the lane set changes after the plugin loads.
    pub fn build(
        lanes: &AutomationLanes,
        plugins: &PluginMap,
    ) -> Self {
        let mut snap = AutomationSnapshot::default();
        for lane in lanes.values() {
            if !lane.enabled {
                continue;
            }
            match lane.target {
                AutomationTarget::PluginParam { instance, param_id } => {
                    let Some(mutex) = plugins.get(&instance) else {
                        continue;
                    };
                    // Engine thread: a brief blocking lock (spin +
                    // back-off) is fine and matches the bounce path.
                    let inst = crate::engine::try_lock_with_backoff(mutex);
                    let range = inst
                        .0
                        .query_params()
                        .into_iter()
                        .find(|p| p.id == param_id)
                        .map(|p| (p.min_value, p.max_value));
                    drop(inst);
                    let Some((min, max)) = range else {
                        continue;
                    };
                    snap.plugin_params.entry(instance).or_default().push(
                        ResolvedParamLane {
                            param_id,
                            lane: lane.clone(),
                            min,
                            max,
                        },
                    );
                }
                _ => {
                    snap.mix_lanes.insert(lane.target.clone(), lane.clone());
                }
            }
        }
        snap
    }
}

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
    let target = lane.target.clone();
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

/// Control-rate cadence for the live automated-value emission (doc #162
/// §2, todo #377): at most one `AutomatedValue` burst per target every
/// ~30 ms while the transport rolls. The engine loop already wakes every
/// ~16 ms, so this is enforced with a wall-clock check, not a busy wait.
pub const AUTOMATED_VALUE_THROTTLE: Duration = Duration::from_millis(30);

/// Smallest change in a normalized lane value (`0.0..=1.0`) that warrants
/// a fresh `AutomatedValue`. Below this the fader/knob tint wouldn't
/// visibly move, so the emission is suppressed — the same "only when the
/// value moves perceptibly" rule the MIDI-clock tempo tracker applies, so
/// a flat lane region doesn't spam the event queue every tick.
pub const AUTOMATED_VALUE_EPSILON: f32 = 1.0 / 512.0;

/// Engine-thread bookkeeping for the throttled live automated-value
/// emission (todo #377). Remembers the last normalized value sent per
/// target so a steady (flat) lane region emits once and then falls quiet,
/// and reuses a scratch buffer so collecting each ~30 ms batch doesn't
/// allocate. Lives on the engine control thread, never the audio callback.
#[derive(Debug, Default)]
pub struct LiveValueEmitter {
    last_sent: HashMap<AutomationTarget, f32>,
    scratch: Vec<(AutomationTarget, f32)>,
}

impl LiveValueEmitter {
    /// Sample every **enabled** (Read-on) lane at `frame` and return the
    /// `(target, value_norm)` pairs whose normalized value moved at least
    /// [`AUTOMATED_VALUE_EPSILON`] since the previous emission, updating the
    /// per-target memo in place. Read-disabled lanes are skipped entirely
    /// (the app keeps showing the static value). The returned slice borrows
    /// the reused scratch buffer, so it's valid until the next call.
    pub fn poll(
        &mut self,
        lanes: &AutomationLanes,
        frame: u64,
    ) -> &[(AutomationTarget, f32)] {
        self.scratch.clear();
        for lane in lanes.values() {
            if !lane.enabled {
                continue;
            }
            let value = lane.sample(frame);
            let moved = match self.last_sent.get(&lane.target) {
                Some(prev) => (prev - value).abs() >= AUTOMATED_VALUE_EPSILON,
                None => true,
            };
            if moved {
                self.last_sent.insert(lane.target.clone(), value);
                self.scratch.push((lane.target.clone(), value));
            }
        }
        &self.scratch
    }

    /// Whether anything has been emitted since the last [`Self::reset`].
    pub fn is_idle(&self) -> bool {
        self.last_sent.is_empty()
    }

    /// Forget every memoized value so the next [`Self::poll`] re-emits a
    /// fresh value for each enabled lane. Called when the transport stops
    /// or pauses, so a later replay — possibly from the very same frame —
    /// re-tints the controls instead of being suppressed as "unchanged".
    pub fn reset(&mut self) {
        self.last_sent.clear();
    }
}
