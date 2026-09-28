//! `automation.add_points` — insert points into a lane (creating it when
//! absent), upserting a point already on an occupied frame (D4;
//! automation-control-api.md §4.4).

use super::edit::{commit_points, current_lane, resolve_points};
use super::target::resolve_write_target;
use crate::message::Message;
use crate::update::control::reply::{reject, success};
use crate::Resonance;
use iced::Task;
use resonance_common::Breakpoint;
use resonance_control::methods::automation::AddPointsParams;
use resonance_control::{Request, Response};

/// Resolve the target and every input point first (a bad point, or two
/// inputs on the same frame, refuses the whole call before anything is
/// dispatched — [`resolve_points`]), merge them onto the lane's existing
/// points (a point already on an input's frame is replaced in place and
/// counted in `replaced`), then commit the complete list as one
/// `SetLane`.
pub(super) fn add_points(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: AddPointsParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let resolved = match resolve_write_target(app, &params.target) {
        Ok(resolved) => resolved,
        Err(e) => return reject(request, e),
    };
    let inserted = match resolve_points(app, &resolved, &params.points, params.normalized) {
        Ok(points) => points,
        Err(e) => return reject(request, e),
    };
    let mut merged: Vec<Breakpoint> = current_lane(app, &resolved.target)
        .map(|lane| lane.points.clone())
        .unwrap_or_default();
    let mut replaced = 0u32;
    for point in inserted {
        match merged
            .iter_mut()
            .find(|existing| existing.time_frames == point.time_frames)
        {
            Some(slot) => {
                *slot = point;
                replaced += 1;
            }
            None => merged.push(point),
        }
    }
    merged.sort_by_key(|p| p.time_frames);
    match commit_points(app, &resolved.target, merged, None) {
        Ok((mut result, task)) => {
            result.replaced = Some(replaced);
            (success(request, &result), task)
        }
        Err(e) => reject(request, e),
    }
}
