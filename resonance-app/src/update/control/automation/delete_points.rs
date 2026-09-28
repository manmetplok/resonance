//! `automation.delete_points` — delete points by index or by half-open
//! range (automation-control-api.md §4.4). Emptying the lane removes it
//! and needs `confirm`; the lane is left intact when it is refused.

use super::edit::{
    commit_points, commit_removal, confirm_removal_summary, current_lane, no_lane_error,
    resolve_range,
};
use super::target::resolve_lane_or_target;
use super::view::lane_view_for;
use crate::message::Message;
use crate::update::control::reply::{mutation_ack, reject, success};
use crate::Resonance;
use iced::Task;
use resonance_common::AutomationLane;
use resonance_control::methods::automation::{DeletePointsParams, LaneEditResult};
use resonance_control::{Request, Response, RpcError};
use std::collections::HashSet;

pub(super) fn delete_points(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: DeletePointsParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if params.indices.is_some() == params.range.is_some() {
        return reject(
            request,
            RpcError::invalid_params(
                "give exactly one of indices[] or range{start, end} to select the points to \
                 delete",
            ),
        );
    }
    let target = match resolve_lane_or_target(app, params.lane_id, &params.target) {
        Ok(target) => target,
        Err(e) => return reject(request, e),
    };
    let Some(lane) = current_lane(app, &target).cloned() else {
        return reject(request, no_lane_error(params.lane_id));
    };
    let to_delete = match indices_to_delete(app, &lane, &params) {
        Ok(set) => set,
        Err(e) => return reject(request, e),
    };
    let deleted = to_delete.len() as u32;
    if to_delete.is_empty() {
        // Nothing matched: an ack with the lane unchanged, no dispatch —
        // no edit happened, so no revision bump either.
        let result = LaneEditResult {
            revision: mutation_ack(app).revision,
            lane: lane_view_for(app, &target),
            removed: false,
            replaced: None,
            deleted: Some(0),
            seed: None,
        };
        return (success(request, &result), Task::none());
    }
    let remaining: Vec<_> = lane
        .points
        .iter()
        .enumerate()
        .filter(|(index, _)| !to_delete.contains(index))
        .map(|(_, point)| point.clone())
        .collect();
    if remaining.is_empty() {
        if !params.confirm {
            return reject(
                request,
                RpcError::needs_confirmation(confirm_removal_summary(app, &lane)),
            );
        }
        let (mut result, task) = commit_removal(app, &target);
        result.deleted = Some(deleted);
        return (success(request, &result), task);
    }
    match commit_points(app, &target, remaining, None) {
        Ok((mut result, task)) => {
            result.deleted = Some(deleted);
            (success(request, &result), task)
        }
        Err(e) => reject(request, e),
    }
}

/// The whole-lane point indices `params` selects, validated against
/// `lane`. An out-of-range index in `indices` is rejected naming the
/// lane's point count; a `range` that matches nothing is not an error —
/// [`delete_points`] acks with `deleted: 0`.
fn indices_to_delete(
    app: &Resonance,
    lane: &AutomationLane,
    params: &DeletePointsParams,
) -> Result<HashSet<usize>, RpcError> {
    if let Some(indices) = &params.indices {
        let count = lane.points.len();
        let mut set = HashSet::with_capacity(indices.len());
        for &index in indices {
            let index = index as usize;
            if index >= count {
                return Err(RpcError::invalid_params(format!(
                    "index {index} is past the lane's {count} point(s)"
                )));
            }
            set.insert(index);
        }
        return Ok(set);
    }
    let range = params
        .range
        .as_ref()
        .expect("indices xor range checked by the caller");
    let (start, end) = resolve_range(app, range)?;
    Ok(lane
        .points
        .iter()
        .enumerate()
        .filter(|(_, point)| point.time_frames >= start && point.time_frames < end)
        .map(|(index, _)| index)
        .collect())
}
