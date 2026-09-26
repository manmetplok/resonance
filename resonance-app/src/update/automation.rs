//! Update handlers for parameter-automation lane edits (architecture doc
//! #162 §3, todo #380, epic #14).
//!
//! Each edit mutates the app-side [`crate::state::AutomationState`]
//! optimistically — so the view reflects the change this frame — and
//! sends the matching `AudioCommand` so the engine's authoritative lanes
//! stay in lock-step. The engine echoes `AutomationLaneChanged` /
//! `AutomationLaneCleared`, which the one-way mirror in
//! `engine_events::automation` re-applies idempotently.
//!
//! Lanes are stored whole-lane-replace (one per target), matching the
//! engine model, so every breakpoint edit re-sends the entire lane via
//! `SetAutomationLane`. Undo capture is handled generically by the
//! classifier in `undo.rs`: the discrete edits are atomic `Record`
//! entries; the breakpoint drag opens a `Begin`/`Commit` transaction so
//! it coalesces into one entry.

use iced::Task;
use resonance_audio::types::{AudioCommand, TrackId};
use resonance_common::{
    plugin_param_to_lane_value, real_to_lane_value, AutomationLane, AutomationTarget, Breakpoint,
    CurveKind,
};

use crate::message::Message;
use crate::Resonance;

/// Edits to parameter-automation lanes (architecture doc #162 §3, epic
/// #14). Every variant maps to one mutation of the app-side
/// [`crate::state::AutomationState`] plus the matching engine command;
/// the discrete edits are atomic undo entries while the breakpoint drag
/// coalesces into one (see the `Begin`/`Commit` classification in
/// `undo.rs`). The `target` identifies which lane is edited; the
/// parameter-picker UI that *chooses* the target lives in todo #383 and
/// the canvas hit-testing that emits the breakpoint edits in todo #382 —
/// this enum is the shared edit vocabulary both dispatch through.
#[derive(Debug, Clone)]
pub enum AutomationMessage {
    /// Create a lane for `target`, seeded with a single breakpoint at
    /// frame 0 holding the target's current static value so turning Read
    /// on doesn't jump the parameter. No-op when a lane already exists.
    AddLane(AutomationTarget),
    /// Remove the lane for `target` entirely (Read flag, breakpoints and
    /// any live tint). No-op when no lane exists.
    RemoveLane(AutomationTarget),
    /// Flip the lane's Read flag (`AutomationLane::enabled`) without
    /// discarding its breakpoints. No-op when no lane exists.
    ToggleRead(AutomationTarget),
    /// Insert a breakpoint at `time_frames` with normalized `value` and
    /// `curve`. Creates the lane first when absent.
    AddBreakpoint {
        target: AutomationTarget,
        time_frames: u64,
        value: f32,
        curve: CurveKind,
    },
    /// Delete the breakpoint at `index` in the lane's time-sorted point
    /// list. Removing the last point clears the lane (an enabled empty
    /// lane would otherwise force the value to its floor).
    DeleteBreakpoint {
        target: AutomationTarget,
        index: usize,
    },
    /// Set the curve kind on the breakpoint at `index`.
    SetCurveKind {
        target: AutomationTarget,
        index: usize,
        curve: CurveKind,
    },
    /// Begin a breakpoint drag: opens an undo transaction so the whole
    /// drag collapses into a single entry. Carries the grabbed `index`
    /// for the caller's drag-state bookkeeping (todo #382).
    StartBreakpointDrag {
        target: AutomationTarget,
        index: usize,
    },
    /// Move the dragged breakpoint to a new `time_frames` + `value`
    /// (mid-gesture; not itself an undo entry). Re-sorts the lane after
    /// the move so the time invariant holds.
    DragBreakpoint {
        target: AutomationTarget,
        index: usize,
        time_frames: u64,
        value: f32,
    },
    /// Commit the breakpoint drag opened by [`Self::StartBreakpointDrag`].
    EndBreakpointDrag,
    /// Cycle which of `track`'s lanes the Arrange overlay band shows: the
    /// lane after the currently shown one in `(target_priority, param-id)`
    /// order, wrapping (todo #1095). Emitted by a click on the band's
    /// parameter-label chip. Mutates only the transient
    /// `AutomationState::lane_selection` view state — no engine command,
    /// no persistence, no undo entry.
    CycleTrackLane(TrackId),
    /// Expand / collapse a track's automation lanes into dedicated slim
    /// arrange sub-rows (doc #256, todo #1096). Pure transient view state
    /// (`ClipInteractionState::automation_expanded_tracks`): no engine
    /// command, no undo entry, no persistence.
    ToggleTrackExpanded(TrackId),
}

pub fn handle(r: &mut Resonance, m: AutomationMessage) -> Task<Message> {
    match m {
        AutomationMessage::AddLane(target) => add_lane(r, target),
        AutomationMessage::RemoveLane(target) => remove_lane(r, target),
        AutomationMessage::ToggleRead(target) => toggle_read(r, target),
        AutomationMessage::AddBreakpoint {
            target,
            time_frames,
            value,
            curve,
        } => add_breakpoint(r, target, time_frames, value, curve),
        AutomationMessage::DeleteBreakpoint { target, index } => {
            delete_breakpoint(r, target, index)
        }
        AutomationMessage::SetCurveKind {
            target,
            index,
            curve,
        } => set_curve_kind(r, target, index, curve),
        AutomationMessage::DragBreakpoint {
            target,
            index,
            time_frames,
            value,
        } => drag_breakpoint(r, target, index, time_frames, value),
        // The drag's pre-gesture snapshot (Begin) and commit (Commit) are
        // taken by the undo classifier; the handler has nothing to mutate
        // at the gesture boundaries.
        AutomationMessage::StartBreakpointDrag { .. } | AutomationMessage::EndBreakpointDrag => {}
        AutomationMessage::CycleTrackLane(track_id) => cycle_track_lane(r, track_id),
        // Transient view state only (doc #256, todo #1096): flips whether
        // the track's automation lanes show as dedicated arrange sub-rows.
        // No engine command, no undo (classified `Skip`), no persistence.
        AutomationMessage::ToggleTrackExpanded(track_id) => {
            let expanded = &mut r.interaction.automation_expanded_tracks;
            if !expanded.remove(&track_id) {
                expanded.insert(track_id);
            }
        }
    }
    Task::none()
}

/// Advance which of `track_id`'s lanes the Arrange overlay shows (todo
/// #1095): the next lane in `(target_priority, param-id)` order after the
/// currently shown one, wrapping. Pure view state — writes only the
/// transient `AutomationState::lane_selection` map (no engine command, no
/// undo, not persisted). No-op when the track is gone or has no lanes.
fn cycle_track_lane(r: &mut Resonance, track_id: resonance_audio::types::TrackId) {
    let next = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .and_then(|track| {
            crate::view::timeline::automation::next_lane_id_for_track(&r.automation, track)
        });
    if let Some(next) = next {
        r.automation.lane_selection.insert(track_id, next);
    }
}

// ---------------------------------------------------------------------
// Lane-level edits
// ---------------------------------------------------------------------

/// Create a lane for `target` seeded with one breakpoint at frame 0 equal
/// to the target's current static value, so a freshly-added (Read-on) lane
/// is flat at the value the user already hears. No-op when a lane exists.
fn add_lane(r: &mut Resonance, target: AutomationTarget) {
    if r.automation.lanes.contains_key(&target) {
        return;
    }
    let value = current_static_lane_value(r, &target);
    let id = r.automation.alloc_lane_id();
    let lane = AutomationLane::new(
        id,
        target,
        vec![Breakpoint::new(0, value, CurveKind::default())],
    );
    store_lane(r, lane);
}

/// Remove the lane (and any transient live tint) for `target`.
fn remove_lane(r: &mut Resonance, target: AutomationTarget) {
    if r.automation.lanes.remove(&target).is_some() {
        r.automation.live_values.remove(&target);
        let _ = r.engine.send(AudioCommand::ClearAutomationLane { target });
    }
}

/// Flip the lane's Read flag. Sends the dedicated `SetAutomationReadEnabled`
/// command (cheaper than a whole-lane replace) and mirrors the flag locally.
fn toggle_read(r: &mut Resonance, target: AutomationTarget) {
    let Some(lane) = r.automation.lanes.get_mut(&target) else {
        return;
    };
    lane.enabled = !lane.enabled;
    let enabled = lane.enabled;
    let _ = r
        .engine
        .send(AudioCommand::SetAutomationReadEnabled { target, enabled });
}

// ---------------------------------------------------------------------
// Breakpoint-level edits
// ---------------------------------------------------------------------

fn add_breakpoint(
    r: &mut Resonance,
    target: AutomationTarget,
    time_frames: u64,
    value: f32,
    curve: CurveKind,
) {
    ensure_lane(r, &target);
    // `ensure_lane` guarantees the entry exists.
    let lane = r.automation.lanes.get_mut(&target).expect("lane ensured");
    lane.insert_point(Breakpoint::new(time_frames, value, curve));
    let lane = lane.clone();
    store_lane(r, lane);
}

fn delete_breakpoint(r: &mut Resonance, target: AutomationTarget, index: usize) {
    let became_empty = {
        let Some(lane) = r.automation.lanes.get_mut(&target) else {
            return;
        };
        if index >= lane.points.len() {
            return;
        }
        lane.points.remove(index);
        lane.points.is_empty()
    };
    if became_empty {
        // An enabled lane with no points samples to its floor, silencing
        // the target — so a lane that loses its last point is cleared
        // outright rather than left empty.
        remove_lane(r, target);
    } else {
        let lane = r.automation.lanes.get(&target).expect("present").clone();
        store_lane(r, lane);
    }
}

fn set_curve_kind(r: &mut Resonance, target: AutomationTarget, index: usize, curve: CurveKind) {
    let lane = {
        let Some(lane) = r.automation.lanes.get_mut(&target) else {
            return;
        };
        let Some(bp) = lane.points.get_mut(index) else {
            return;
        };
        if bp.curve == curve {
            return;
        }
        bp.curve = curve;
        lane.clone()
    };
    store_lane(r, lane);
}

fn drag_breakpoint(
    r: &mut Resonance,
    target: AutomationTarget,
    index: usize,
    time_frames: u64,
    value: f32,
) {
    let lane = {
        let Some(lane) = r.automation.lanes.get_mut(&target) else {
            return;
        };
        if index >= lane.points.len() {
            return;
        }
        // Keep the dragged point's curve; only its position moves.
        let curve = lane.points[index].curve;
        lane.points[index] = Breakpoint::new(time_frames, value, curve);
        lane.sort_points();
        lane.clone()
    };
    store_lane(r, lane);
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

/// Insert an empty lane for `target` when none exists yet (the caller
/// immediately adds a breakpoint, so the transient empty state never
/// reaches the engine).
fn ensure_lane(r: &mut Resonance, target: &AutomationTarget) {
    if !r.automation.lanes.contains_key(target) {
        let id = r.automation.alloc_lane_id();
        r.automation.lanes.insert(
            target.clone(),
            AutomationLane::new(id, target.clone(), Vec::new()),
        );
    }
}

/// Mirror a lane into app state and push it to the engine (whole-lane
/// replace).
fn store_lane(r: &mut Resonance, lane: AutomationLane) {
    r.automation.lanes.insert(lane.target.clone(), lane.clone());
    let _ = r.engine.send(AudioCommand::SetAutomationLane { lane });
}

/// The normalized `0.0..=1.0` lane value equal to the target's current
/// static value, read from live GUI state (never the engine, per the
/// command/event boundary). Used to seed a newly-added lane so it starts
/// flat at the value already in effect.
fn current_static_lane_value(r: &Resonance, target: &AutomationTarget) -> f32 {
    use AutomationTarget::*;
    let real = match target {
        TrackGain(id) => track_field(r, *id, |t| t.volume),
        TrackPan(id) => track_field(r, *id, |t| t.pan),
        TrackMute(id) => track_field(r, *id, |t| if t.muted { 1.0 } else { 0.0 }),
        BusGain(id) => bus_field(r, *id, |b| b.volume),
        BusPan(id) => bus_field(r, *id, |b| b.pan),
        BusMute(id) => bus_field(r, *id, |b| if b.muted { 1.0 } else { 0.0 }),
        MasterGain => r.master_volume,
        // Plugin params live in a per-plugin range, not the fixed ranges
        // `real_to_lane_value` knows, so normalize with the param's own
        // `min..=max` and return directly.
        PluginParam { instance, param_id } => {
            return current_plugin_param_lane_value(r, *instance, *param_id)
        }
        // Device params are normalized by the engine via the device
        // definition's binding mapping (not the fixed ranges here), so seed
        // a freshly-added lane at its floor — the engine corrects on replay.
        DeviceParam { .. } => 0.0,
    };
    real_to_lane_value(target, real)
}

fn track_field(r: &Resonance, id: u64, f: impl Fn(&crate::state::TrackState) -> f32) -> f32 {
    r.registry
        .tracks
        .iter()
        .find(|t| t.id == id)
        .map(f)
        .unwrap_or(0.0)
}

fn bus_field(r: &Resonance, id: u64, f: impl Fn(&crate::state::BusState) -> f32) -> f32 {
    r.registry
        .busses
        .iter()
        .find(|b| b.id == id)
        .map(f)
        .unwrap_or(0.0)
}

/// Normalized current value of one CLAP parameter, found from the
/// inspector's per-slot `ParamInfo` cache (the same metadata todo #383's
/// picker uses). Falls back to `0.0` when the plugin or param isn't found.
fn current_plugin_param_lane_value(r: &Resonance, instance: u64, param_id: u32) -> f32 {
    let slots = r
        .registry
        .tracks
        .iter()
        .flat_map(|t| t.plugins.iter())
        .chain(r.registry.busses.iter().flat_map(|b| b.plugins.iter()))
        .chain(r.master_plugins.iter());
    for slot in slots {
        if slot.instance_id != instance {
            continue;
        }
        if let Some(p) = slot.params.iter().find(|p| p.id == param_id) {
            return plugin_param_to_lane_value(p.current_value, p.min_value, p.max_value);
        }
    }
    0.0
}
