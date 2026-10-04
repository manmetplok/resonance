//! App-side mirror of the engine's parameter-automation lanes, plus the
//! transient live automated values shown on faders/knobs during playback
//! (architecture doc #162 §3, epic #14).
//!
//! The engine owns the authoritative lanes; this state is a one-way mirror
//! reconstructed from `AutomationLaneChanged` / `AutomationLaneCleared`
//! (e.g. after a project load replays `SetAutomationLane`). It is keyed by
//! [`AutomationTarget`] — one lane per target, matching the engine's
//! whole-lane-replace storage — so a lookup by target is O(1) for the view
//! and update layers.

use std::collections::HashMap;

use resonance_common::{AutomationLane, AutomationTarget, LaneId, TrackId};

use crate::state::TrackState;

/// GUI-side automation state, mirrored one-way from engine events.
#[derive(Debug, Clone, Default)]
pub struct AutomationState {
    /// Every lane the engine holds, keyed by its target. One lane per
    /// target. Reconstructed from `AutomationLaneChanged` (insert/replace)
    /// and `AutomationLaneCleared` (remove).
    pub lanes: HashMap<AutomationTarget, AutomationLane>,
    /// Most recent throttled automated value per target, from
    /// `AutomatedValue`. Transient — used to tint the fader/knob while
    /// Read is on; never persisted and not part of undo. Cleared for a
    /// target when its lane is removed so a stale tint can't outlive the
    /// lane.
    pub live_values: HashMap<AutomationTarget, f32>,
    /// Transient per-track override of which lane the Arrange overlay
    /// draws (todo #1095): the chip click cycles this through every lane
    /// that targets the track. Pure view state — never persisted, never
    /// sent to the engine, and not part of undo (the undo snapshot
    /// captures only [`Self::lanes`]). A stale entry (its lane removed or
    /// re-homed) is ignored by the view, which falls back to the
    /// priority-based default silently.
    pub lane_selection: HashMap<TrackId, LaneId>,
    /// Monotonic source of [`LaneId`]s for lanes the *app* creates (a
    /// "pick parameter → add lane" edit). The engine stores lanes keyed by
    /// target and echoes the id back unchanged, so the app owns id
    /// allocation. Never reused — a fresh id per `AddLane` keeps ids unique
    /// across a session even after undo re-creates a lane.
    next_lane_id: LaneId,
}

impl AutomationState {
    /// Allocate the next unique lane id (1-based; `0` means "none
    /// allocated yet", matching the `Default` value).
    pub fn alloc_lane_id(&mut self) -> LaneId {
        self.next_lane_id += 1;
        self.next_lane_id
    }

    /// The live automated value (normalized `0.0..=1.0`) the mixer should
    /// tint `target`'s fader/knob with, or `None` when nothing is driving
    /// it: no lane, the lane's Read flag is off, or no throttled
    /// `AutomatedValue` has arrived yet (playback isn't currently
    /// automating it). The view maps the returned normalized value to the
    /// target's real range for display (todo #383).
    pub fn live_value(&self, target: AutomationTarget) -> Option<f32> {
        let lane = self.lanes.get(&target)?;
        if !lane.enabled {
            return None;
        }
        self.live_values.get(&target).copied()
    }

    /// Replace every mirrored lane with the set loaded from a project file,
    /// keyed by target, and bump the id allocator past every loaded lane id
    /// so app-created lanes after the load never collide with persisted
    /// ones. Clears the transient live-value tints (they belong to the old
    /// project). Used by the project-load replay (architecture doc #162 §3).
    pub fn load_lanes(&mut self, lanes: impl IntoIterator<Item = AutomationLane>) {
        self.lanes.clear();
        self.live_values.clear();
        // The chip-cycle lane selection belongs to the old project's lanes;
        // a fresh load starts from the priority-based defaults again.
        self.lane_selection.clear();
        for lane in lanes {
            self.next_lane_id = self.next_lane_id.max(lane.id);
            self.lanes.insert(lane.target.clone(), lane);
        }
    }

    /// Bump the id allocator past every id currently in `lanes` so an
    /// app-created lane after a wholesale install (project-load replay or
    /// undo restore, both of which set `lanes` directly rather than via
    /// [`load_lanes`]) never reissues a persisted id. Monotonic and
    /// idempotent — `next_lane_id` only ever grows.
    pub fn bump_allocator_past_lanes(&mut self) {
        if let Some(max) = self.lanes.values().map(|l| l.id).max() {
            self.next_lane_id = self.next_lane_id.max(max);
        }
    }
}

/// Display priority of an automation target on its track's arrange row:
/// gain, pan, mute, then device params, then plugin params by param id.
/// Bus/master targets never belong to an arrange track row.
pub fn target_priority(target: AutomationTarget) -> u32 {
    match target {
        AutomationTarget::TrackGain(_) => 0,
        AutomationTarget::TrackPan(_) => 1,
        AutomationTarget::TrackMute(_) => 2,
        // Device params are what the user automated deliberately on an
        // external-instrument track, so they outrank generic plugin params.
        // Ties between several device lanes are broken deterministically by
        // `param_id` in `track_lanes_sorted`.
        AutomationTarget::DeviceParam { .. } => 5,
        AutomationTarget::PluginParam { param_id, .. } => 10u32.saturating_add(param_id),
        _ => u32::MAX,
    }
}

/// Whether `target` drives one of `track`'s parameters: its own gain/pan/
/// mute, a CLAP param on a plugin instance hosted by the track, or a device
/// param on the track's external instrument.
pub fn target_belongs_to_track(target: &AutomationTarget, track: &TrackState) -> bool {
    match target {
        AutomationTarget::TrackGain(id)
        | AutomationTarget::TrackPan(id)
        | AutomationTarget::TrackMute(id) => *id == track.id,
        AutomationTarget::PluginParam { instance, .. } => {
            track.plugins.iter().any(|p| p.instance_id == *instance)
        }
        AutomationTarget::DeviceParam { track: id, .. } => *id == track.id,
        _ => false,
    }
}

/// Every lane that belongs to `track`, sorted by `(target_priority,
/// device-param id)` — gain, pan, mute, device params (lexicographic by
/// param id), then plugin params. This is both the order the default pick
/// scans (the first entry is the "primary" lane) and the cycle order of the
/// chip click (todo #1095), so the two can never disagree. Shared by the
/// arrange-row layout (`state::arrange_layout`) and the timeline's
/// automation overlay, which is why it lives here and not in the view
/// (ARCH2-05).
pub fn track_lanes_sorted<'l>(
    automation: &'l AutomationState,
    track: &TrackState,
) -> Vec<&'l AutomationLane> {
    let mut lanes: Vec<&'l AutomationLane> = automation
        .lanes
        .values()
        .filter(|lane| target_belongs_to_track(&lane.target, track))
        .collect();
    lanes.sort_by_key(|lane| {
        // Copy the inner `&'l` ref out so the tie-break `&str` borrows
        // from the lane itself, not the closure-local double reference.
        let lane: &'l AutomationLane = lane;
        let tie = match &lane.target {
            AutomationTarget::DeviceParam { param_id, .. } => param_id.as_str(),
            _ => "",
        };
        // Final lane-id tie-break: two plugin-param lanes with the same
        // param id on different instances would otherwise inherit the
        // HashMap's nondeterministic iteration order — and the arrange
        // lane-row stacking (`ArrangeAutomationRows::collect`) delegates
        // here, so the cycle order, the default pick, and row 0 of an
        // expanded stack all agree.
        (target_priority(lane.target.clone()), tie, lane.id)
    });
    lanes
}
