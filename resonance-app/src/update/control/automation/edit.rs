//! The write half every mutating `automation.*` handler shares
//! (automation-control-api.md §4.4).
//!
//! A handler resolves its target ([`super::target::resolve_write_target`]),
//! computes the lane's complete new point list as a pure function —
//! [`resolve_points`] turns wire points into breakpoints, [`current_lane`]
//! is what is there now — and hands it to [`commit_points`] (or
//! [`commit_removal`]), which dispatches exactly ONE
//! `AutomationMessage` through `update()` and builds the
//! `{revision, lane}` reply. One call, one message, one undo entry.

use super::target::ResolvedTarget;
use super::view::{lane_view_for, model_curve};
use crate::message::{AutomationMessage, Message};
use crate::update::control::reply::mutation_ack;
use crate::update::control::run_via_update;
use crate::update::control::transport::resolve_position;
use crate::Resonance;
use iced::Task;
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint};
use resonance_control::methods::automation::{
    LaneEditResult, PointRange, PointSpec, MAX_POINTS_PER_CALL, MAX_POINTS_PER_LANE,
};
use resonance_control::{check_max_bars, PositionSpec, RpcError};

/// The lane on `target` now, if any.
pub(in crate::update::control) fn current_lane<'a>(
    app: &'a Resonance,
    target: &AutomationTarget,
) -> Option<&'a AutomationLane> {
    app.automation.lanes.get(target)
}

/// A wire position as a sample: the bar goes through the shared
/// `check_max_bars` guard, then the one resolver every positional input
/// uses (`transport::resolve_position`, meter-aware, 1-based).
pub(in crate::update::control) fn resolve_point_position(
    app: &Resonance,
    spec: &PositionSpec,
) -> Result<u64, RpcError> {
    if let Some(bar) = spec.bar {
        check_max_bars("bar", bar)?;
    }
    resolve_position(app, spec)
}

/// A wire range as a half-open `[start, end)` sample window: an omitted
/// start is sample 0, an omitted end runs past every point.
pub(in crate::update::control) fn resolve_range(
    app: &Resonance,
    range: &PointRange,
) -> Result<(u64, u64), RpcError> {
    let start = match &range.start {
        Some(spec) => resolve_point_position(app, spec)?,
        None => 0,
    };
    let end = match &range.end {
        Some(spec) => resolve_point_position(app, spec)?,
        None => u64::MAX,
    };
    if end <= start {
        return Err(RpcError::invalid_params(format!(
            "range end (sample {end}) must lie after its start (sample {start})"
        )));
    }
    Ok((start, end))
}

/// Refuse writing more than [`MAX_POINTS_PER_CALL`] points in one call.
pub(in crate::update::control) fn check_call_limit(count: usize) -> Result<(), RpcError> {
    if count > MAX_POINTS_PER_CALL {
        return Err(RpcError::invalid_params(format!(
            "{count} points in one call is past the limit of {MAX_POINTS_PER_CALL}; write \
             fewer points per call (for automation.shape, a lower resolution)"
        )));
    }
    Ok(())
}

/// Wire points → breakpoints on `resolved`, sorted by frame.
///
/// Rejects, naming the offending input: more than
/// [`MAX_POINTS_PER_CALL`] points, a bad position, a bad value, and two
/// inputs that land on the SAME frame (the model would keep both, and
/// which one wins would depend on input order) — naming both indices.
pub(in crate::update::control) fn resolve_points(
    app: &Resonance,
    resolved: &ResolvedTarget,
    points: &[PointSpec],
    normalized: bool,
) -> Result<Vec<Breakpoint>, RpcError> {
    check_call_limit(points.len())?;
    let mut out: Vec<(usize, Breakpoint)> = Vec::with_capacity(points.len());
    for (i, point) in points.iter().enumerate() {
        let frame = resolve_point_position(app, &point.position)
            .map_err(|e| RpcError::new(e.kind(), format!("points[{i}].position: {}", e.message)))?;
        let value = resolved
            .domain
            .normalize(&resolved.target, &point.value, normalized)
            .map_err(|e| RpcError::new(e.kind(), format!("points[{i}].value: {}", e.message)))?;
        let curve = point
            .curve
            .map(model_curve)
            .unwrap_or_else(|| resolved.domain.default_curve());
        out.push((i, Breakpoint::new(frame, value, curve)));
    }
    // Stable, so equal frames stay in input order for the error below.
    out.sort_by_key(|(_, p)| p.time_frames);
    if let Some(pair) = out
        .windows(2)
        .find(|w| w[0].1.time_frames == w[1].1.time_frames)
    {
        let (a, b) = (pair[0].0, pair[1].0);
        return Err(RpcError::invalid_params(format!(
            "points[{a}] and points[{b}] both resolve to sample {}; a lane holds one point \
             per position",
            pair[0].1.time_frames
        )));
    }
    Ok(out.into_iter().map(|(_, p)| p).collect())
}

/// Replace `resolved`'s lane with `points` — its COMPLETE new point list,
/// sorted by frame and non-empty — by dispatching one
/// `AutomationMessage::SetLane`, and describe the result.
///
/// `enabled` of `None` keeps an existing lane's Read flag; a new lane
/// starts enabled. Refuses a list past [`MAX_POINTS_PER_LANE`]. The
/// returned [`LaneEditResult`] carries the post-edit revision and the
/// lane as it reads back; callers fill in `replaced` / `deleted` / `seed`.
pub(in crate::update::control) fn commit_points(
    app: &mut Resonance,
    resolved: &ResolvedTarget,
    points: Vec<Breakpoint>,
    enabled: Option<bool>,
) -> Result<(LaneEditResult, Task<Message>), RpcError> {
    if points.is_empty() {
        return Err(RpcError::invalid_params(
            "a lane needs at least one point; use automation.remove_lane to delete it",
        ));
    }
    if points.len() > MAX_POINTS_PER_LANE {
        return Err(RpcError::invalid_params(format!(
            "the lane would hold {} points, past the limit of {MAX_POINTS_PER_LANE} per lane; \
             write fewer points (for automation.shape, a lower resolution)",
            points.len()
        )));
    }
    let enabled = enabled
        .or_else(|| current_lane(app, &resolved.target).map(|l| l.enabled))
        .unwrap_or(true);
    let task = run_via_update(
        app,
        Message::Automation(AutomationMessage::SetLane {
            target: resolved.target.clone(),
            points,
            enabled,
        }),
    );
    let result = LaneEditResult {
        revision: mutation_ack(app).revision,
        lane: lane_view_for(app, &resolved.target),
        removed: false,
        replaced: None,
        deleted: None,
        seed: None,
    };
    Ok((result, task))
}

/// Remove `target`'s lane by dispatching one `AutomationMessage::RemoveLane`;
/// the result is `{revision, lane: null, removed: true}`. The caller has
/// already checked the lane exists and any `confirm` gate.
///
/// Unused until `automation.remove_lane` / `delete_points` land (slices
/// A4/A5); it lives here so they share the reply shape.
#[allow(dead_code)]
pub(in crate::update::control) fn commit_removal(
    app: &mut Resonance,
    target: &AutomationTarget,
) -> (LaneEditResult, Task<Message>) {
    let task = run_via_update(
        app,
        Message::Automation(AutomationMessage::RemoveLane(target.clone())),
    );
    let result = LaneEditResult {
        revision: mutation_ack(app).revision,
        lane: lane_view_for(app, target),
        removed: true,
        replaced: None,
        deleted: None,
        seed: None,
    };
    (result, task)
}
