//! `automation.set_enabled` — the lane's Read flag, declaratively
//! (automation-control-api.md §4.4): a no-op ack when the flag already
//! matches, so a call never costs an undo entry or a revision it did not
//! change anything for.

use super::edit::current_lane;
use super::target::resolve_frozen_checked;
use super::view::lane_view_for;
use crate::message::{AutomationMessage, Message};
use crate::update::control::reply::{mutation_ack, reject, success};
use crate::update::control::run_via_update;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::automation::{LaneEditResult, SetEnabledParams};
use resonance_control::{Request, Response, RpcError};

pub(super) fn set_enabled(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetEnabledParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let resolved = match resolve_frozen_checked(app, &params.target) {
        Ok(resolved) => resolved,
        Err(e) => return reject(request, e),
    };
    let Some(lane) = current_lane(app, &resolved.target) else {
        return reject(
            request,
            RpcError::not_found("no automation lane on this target"),
        );
    };
    if lane.enabled == params.enabled {
        // Nothing to dispatch: an ack at the current revision, no bump.
        let result = LaneEditResult {
            revision: mutation_ack(app).revision,
            lane: lane_view_for(app, &resolved.target),
            removed: false,
            replaced: None,
            deleted: None,
            seed: None,
        };
        return (success(request, &result), Task::none());
    }
    let task = run_via_update(
        app,
        Message::Automation(AutomationMessage::ToggleRead(resolved.target.clone())),
    );
    let result = LaneEditResult {
        revision: mutation_ack(app).revision,
        lane: lane_view_for(app, &resolved.target),
        removed: false,
        replaced: None,
        deleted: None,
        seed: None,
    };
    (success(request, &result), task)
}
