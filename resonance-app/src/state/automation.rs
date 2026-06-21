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

use resonance_common::{AutomationLane, AutomationTarget};

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
}
