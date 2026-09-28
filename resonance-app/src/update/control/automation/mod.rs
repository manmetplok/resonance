//! `automation.*` control methods (automation-control-api.md).
//!
//! # Layout
//!
//! | module | role |
//! |---|---|
//! | [`target`] | wire target → model target (+ the frozen-track write rule) |
//! | [`value`] | real ↔ normalized values, via `resonance_common`'s mappings |
//! | [`view`] | lane → wire views, compact summaries, owner / status / order |
//! | [`edit`] | the shared write path: points → breakpoints → ONE `SetLane` |
//! | [`lanes`] | `automation.lanes` (read-only) |
//! | [`set_lane`] | `automation.set_lane` |
//! | [`add_points`] | `automation.add_points` |
//! | [`delete_points`] | `automation.delete_points` |
//! | [`set_enabled`] | `automation.set_enabled` |
//! | [`remove_lane`] | `automation.remove_lane` |
//!
//! Every write computes the lane's complete new point list as a pure
//! function and dispatches exactly one `AutomationMessage` through
//! `update()` (see [`edit`]), so the edit passes the gates, is undoable
//! like a GUI edit, and costs one revision. `automation.shape` (§4.6)
//! already has its wire type in `resonance_control::methods::automation`;
//! it lands as a handler module here, a `try_handle` arm, a `METHODS`
//! entry and its MCP tool.

use crate::message::Message;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::automation as wire;
use resonance_control::{Request, Response};

mod add_points;
mod delete_points;
pub(in crate::update::control) mod edit;
mod lanes;
mod remove_lane;
mod set_enabled;
mod set_lane;
pub(in crate::update::control) mod target;
pub(in crate::update::control) mod value;
pub(in crate::update::control) mod view;

pub(in crate::update::control) use view::{lane_count, lane_summaries};

/// Handle an `automation.*` request, or `None` when `method` belongs to
/// another namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let out = match request.method.as_str() {
        wire::LANES => (lanes::lanes(app, request), Task::none()),
        wire::SET_LANE => set_lane::set_lane(app, request),
        wire::ADD_POINTS => add_points::add_points(app, request),
        wire::DELETE_POINTS => delete_points::delete_points(app, request),
        wire::SET_ENABLED => set_enabled::set_enabled(app, request),
        wire::REMOVE_LANE => remove_lane::remove_lane(app, request),
        _ => return None,
    };
    Some(out)
}
