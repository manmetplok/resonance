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

use resonance_common::{AutomationLane, AutomationTarget, LaneId};

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

    /// Replace every mirrored lane with the set loaded from a project file,
    /// keyed by target, and bump the id allocator past every loaded lane id
    /// so app-created lanes after the load never collide with persisted
    /// ones. Clears the transient live-value tints (they belong to the old
    /// project). Used by the project-load replay (architecture doc #162 §3).
    pub fn load_lanes(&mut self, lanes: impl IntoIterator<Item = AutomationLane>) {
        self.lanes.clear();
        self.live_values.clear();
        for lane in lanes {
            self.next_lane_id = self.next_lane_id.max(lane.id);
            self.lanes.insert(lane.target.clone(), lane);
        }
    }
}
