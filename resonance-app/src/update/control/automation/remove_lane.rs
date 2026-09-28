//! `automation.remove_lane` — delete the whole lane; confirm-gated when
//! it holds points (automation-control-api.md §4.4).

use super::edit::{commit_removal, confirm_removal_summary, current_lane, no_lane_error};
use super::target::resolve_lane_or_target;
use crate::message::Message;
use crate::update::control::reply::{reject, success};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::automation::RemoveLaneParams;
use resonance_control::{Request, Response, RpcError};

pub(super) fn remove_lane(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: RemoveLaneParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let target = match resolve_lane_or_target(app, params.lane_id, &params.target) {
        Ok(target) => target,
        Err(e) => return reject(request, e),
    };
    let Some(lane) = current_lane(app, &target).cloned() else {
        return reject(request, no_lane_error(params.lane_id));
    };
    if !lane.points.is_empty() && !params.confirm {
        return reject(
            request,
            RpcError::needs_confirmation(confirm_removal_summary(app, &lane)),
        );
    }
    let (result, task) = commit_removal(app, &target);
    (success(request, &result), task)
}
