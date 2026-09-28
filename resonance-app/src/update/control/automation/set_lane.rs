//! `automation.set_lane` — create a lane, or replace ALL of its points.

use super::edit::{commit_points, resolve_points};
use super::target::resolve_write_target;
use crate::message::Message;
use crate::update::control::reply::{reject, success};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::automation::SetLaneParams;
use resonance_control::{Request, Response, RpcError};

/// Resolve the target and every point first — a bad point refuses the
/// whole call before anything is dispatched — then commit the complete
/// list as one `SetLane`.
pub(super) fn set_lane(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetLaneParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if params.points.is_empty() {
        return reject(
            request,
            RpcError::invalid_params(
                "points is empty; set_lane replaces ALL of a lane's points, so an empty list \
                 would delete the lane — use automation.remove_lane for that",
            ),
        );
    }
    let resolved = match resolve_write_target(app, &params.target) {
        Ok(resolved) => resolved,
        Err(e) => return reject(request, e),
    };
    let points = match resolve_points(app, &resolved, &params.points, params.normalized) {
        Ok(points) => points,
        Err(e) => return reject(request, e),
    };
    match commit_points(app, &resolved, points, params.enabled) {
        Ok((result, task)) => (success(request, &result), task),
        Err(e) => reject(request, e),
    }
}
