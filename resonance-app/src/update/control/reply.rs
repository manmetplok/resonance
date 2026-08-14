//! The control layer's reply vocabulary (ba todo #1258).
//!
//! Every handler ends the same four ways — a serialized result, an
//! error, a `{revision}` acknowledgement, or an acknowledgement plus the
//! task the routed domain message produced — so those four live here
//! once instead of once per namespace module.
//!
//! In particular [`mutation_ack`] is the ONLY place
//! [`MutationAck`](resonance_control::MutationAck) is constructed. It
//! carries a single field today, which is exactly why the hand-rolled
//! copies this module replaced all agreed: the day it gains a second
//! one, a copy would have kept compiling and quietly replied without it.
//!
//! The `not_found` constructors sit here for the same reason: a wire id
//! that misses should read the same whichever namespace was asked, so
//! "no track with id 7" is one string, not nine.

use crate::message::Message;
use crate::Resonance;
use iced::Task;
use resonance_control::{MutationAck, Request, Response, RpcError};

// ---------------------------------------------------------------------------
// Envelopes
// ---------------------------------------------------------------------------

/// A success reply for `request`; falls back to an internal error if the
/// result fails to serialize (unreachable for well-formed result types).
pub(super) fn success<T: serde::Serialize>(request: &Request, result: &T) -> Response {
    Response::success(request.id.clone(), result).unwrap_or_else(|e| {
        Response::failure(
            Some(request.id.clone()),
            RpcError::internal(format!("failed to encode result: {e}")),
        )
    })
}

/// An error reply for `request`.
pub(super) fn failure(request: &Request, error: RpcError) -> Response {
    Response::failure(Some(request.id.clone()), error)
}

// ---------------------------------------------------------------------------
// Mutation replies
// ---------------------------------------------------------------------------

/// The `{revision}` acknowledgement every mutating reply carries,
/// snapshotting the app's monotonic undoable-transaction counter.
///
/// Construct [`MutationAck`] here and nowhere else.
pub(super) fn mutation_ack(app: &Resonance) -> MutationAck {
    MutationAck {
        revision: app.revision(),
    }
}

/// The [`MutationAck`] reply carrying the post-edit revision.
pub(super) fn ack(app: &Resonance, request: &Request) -> Response {
    success(request, &mutation_ack(app))
}

/// [`ack`] plus the task the routed domain message produced — the shape
/// a dispatching handler returns.
pub(super) fn ack_task(
    app: &Resonance,
    request: &Request,
    task: Task<Message>,
) -> (Response, Task<Message>) {
    (ack(app, request), task)
}

/// [`ack_task`], unless the dispatch left a compose error behind — then
/// that error, as `invalid_params`.
///
/// The compose reducers report failure by parking a message on
/// `compose.last_error` rather than by returning it, so a synthesized
/// `ComposeMessage` "succeeds" structurally even when it did nothing.
/// Every compose-routed namespace (`section.*`, `harmony.*`, `vocal.*`)
/// has to check, and they check identically.
pub(super) fn ack_or_compose_error(
    app: &mut Resonance,
    request: &Request,
    task: Task<Message>,
) -> (Response, Task<Message>) {
    if let Some(error) = app.compose.last_error.take() {
        return reject(request, RpcError::invalid_params(error));
    }
    ack_task(app, request, task)
}

/// An error reply that still satisfies a dispatching handler's
/// `(Response, Task)` signature: nothing ran, so there is no task.
pub(super) fn reject(request: &Request, error: RpcError) -> (Response, Task<Message>) {
    (failure(request, error), Task::none())
}

// ---------------------------------------------------------------------------
// Shared `not_found` vocabulary
// ---------------------------------------------------------------------------

/// "No track with id N" — the miss every track-addressing method
/// reports, worded once.
pub(super) fn no_track(id: u64) -> RpcError {
    RpcError::not_found(format!("no track with id {id}"))
}

/// "No bus with id N". Callers that can also suggest a fix (e.g. routing
/// into a bus that must first be created) build their own richer error.
pub(super) fn no_bus(id: u64) -> RpcError {
    RpcError::not_found(format!("no bus with id {id}"))
}

/// "No MIDI clip with id N" — note editing and MIDI import address the
/// same clips.
pub(super) fn no_midi_clip(id: u64) -> RpcError {
    RpcError::not_found(format!("no MIDI clip with id {id}"))
}

/// "No section definition with id N".
pub(super) fn no_section_definition(id: u64) -> RpcError {
    RpcError::not_found(format!("no section definition with id {id}"))
}

/// "No section placement with id N".
pub(super) fn no_section_placement(id: u64) -> RpcError {
    RpcError::not_found(format!("no section placement with id {id}"))
}

/// [`no_track`] as a full rejection, for the dispatching handlers.
pub(super) fn not_found_track(request: &Request, id: u64) -> (Response, Task<Message>) {
    reject(request, no_track(id))
}

/// [`no_bus`] as a full rejection, for the dispatching handlers.
pub(super) fn not_found_bus(request: &Request, id: u64) -> (Response, Task<Message>) {
    reject(request, no_bus(id))
}
