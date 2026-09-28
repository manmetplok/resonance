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
//!
//! Every write computes the lane's complete new point list as a pure
//! function and dispatches exactly one `AutomationMessage` through
//! `update()` (see [`edit`]), so the edit passes the gates, is undoable
//! like a GUI edit, and costs one revision. The remaining methods of
//! §4.4 (`add_points`, `delete_points`, `set_enabled`, `remove_lane`,
//! `shape`) already have their wire types in
//! `resonance_control::methods::automation`; each lands as a handler
//! module here, a `try_handle` arm, a `METHODS` entry and its MCP tool.

use crate::message::Message;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::automation as wire;
use resonance_control::{Request, Response};

pub(in crate::update::control) mod edit;
mod lanes;
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
        _ => return None,
    };
    Some(out)
}
