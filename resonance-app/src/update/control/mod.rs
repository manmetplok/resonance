//! Control-endpoint request execution (ba doc #265, todo #1147).
//!
//! Handles `Message::Control` on the update loop: connection
//! bookkeeping, the `control.hello` handshake, and the method-dispatch
//! skeleton the namespace todos (#1148 `song.*`, #1149 jobs, #1150
//! `transport.*`, ...) fill in.
//!
//! # Execution model
//!
//! - **Read-only methods** (`song.*`, `job.status`, `control.hello`)
//!   execute against `&Resonance` and never mutate.
//! - **Mutating methods** synthesize existing domain [`Message`] values
//!   and route them through the *full* [`Resonance::update`] path — the
//!   pre-dispatch gates, the frozen-input classifier, `record_undo`,
//!   dispatch, and the transaction commit — via [`run_via_update`]. An AI
//!   edit is therefore a normal, undoable edit, and the resulting
//!   [`Task`]s reach the iced runtime like any other. Handlers never
//!   write engine/project state directly.
//! - Every request gets exactly one reply, sent over the request's
//!   [`ReplySender`](crate::control_socket::ReplySender) to the
//!   connection's writer thread — never blocking the update loop.
//!
//! Requests arrive strictly in socket arrival order (single bridge
//! channel) and run one at a time on the update loop, so they never race
//! the GUI.

use crate::control_socket::{ConnId, ControlMessage, ControlRequest};
use crate::message::Message;
use crate::state::ControlSession;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::control::{HelloParams, HelloResult, HELLO};
use resonance_control::{Request, Response, RpcError, PROTOCOL_VERSION};

mod generate;
mod harmony;
mod job;
mod project;
mod render;
mod section;
mod song;
mod vocal;

pub(crate) use render::mixdown_result;

/// Entry point for `Message::Control`, dispatched from `update.rs`.
pub fn handle(app: &mut Resonance, message: ControlMessage) -> Task<Message> {
    match message {
        ControlMessage::Connected { conn } => {
            app.control.sessions.insert(conn, ControlSession::default());
            Task::none()
        }
        ControlMessage::Disconnected { conn } => {
            app.control.sessions.remove(&conn);
            // Drop the connection's jobs (todo #1149): nobody can query
            // them anymore, and a reader blocked in `job.wait` on one
            // of them resolves to `not_found`.
            app.control.jobs.on_disconnect(conn);
            Task::none()
        }
        ControlMessage::Request(request) => {
            let ControlRequest {
                conn,
                request,
                reply,
            } = request;
            let (response, task) = execute(app, conn, &request);
            reply.send(response);
            task
        }
    }
}

/// Execute one request against the app, returning the reply plus any
/// [`Task`] produced by a routed domain message. Split from [`handle`]
/// so integration tests can drive the dispatch with synthesized
/// requests and assert on the raw [`Response`].
pub fn execute(
    app: &mut Resonance,
    conn: ConnId,
    request: &Request,
) -> (Response, Task<Message>) {
    let method = request.method.as_str();

    // The handshake is always answered, even on an incompatible
    // connection — it's how the client learns what the server speaks.
    if method == HELLO {
        return (hello(app, conn, request), Task::none());
    }

    // Doc #265: reject requests from clients that declared an
    // incompatible major version.
    if app
        .control
        .sessions
        .get(&conn)
        .is_some_and(|s| s.incompatible)
    {
        return (
            failure(
                request,
                RpcError::unsupported(format!(
                    "connection declared incompatible protocol version; \
                     server speaks {PROTOCOL_VERSION}"
                )),
            ),
            Task::none(),
        );
    }

    // Read-only introspection (todo #1148): the `song.*` views plus the
    // plugin catalog. Executed against `&Resonance` — never mutates.
    if let Some(response) = song::try_handle(app, request) {
        return (response, Task::none());
    }

    // Job registry (todo #1149): `job.status` (and the update-loop
    // fallback for `job.wait` — the socket transport serves the
    // blocking form on its reader threads).
    if let Some(response) = job::try_handle(app, request) {
        return (response, Task::none());
    }

    // Project lifecycle (todo #1151): new/open/save/save-as with explicit
    // paths, as jobs. Mutating — the returned task must reach the runtime.
    if let Some((response, task)) = project::try_handle(app, conn, request) {
        return (response, task);
    }

    // Mutating compose namespaces (todo #1153): `section.*` sections +
    // placements, `harmony.*` chords + progression apply. Both synthesize
    // ComposeMessage values routed through `run_via_update`.
    if let Some(handled) = section::try_handle(app, request) {
        return handled;
    }
    if let Some(handled) = harmony::try_handle(app, request) {
        return handled;
    }
    // Generators (todo #1154): generate.part / generate.drums into a
    // section + track.
    if let Some(handled) = generate::try_handle(app, request) {
        return handled;
    }
    // Vocals (todo #1156): lyrics, pronunciation, and the SVS render job.
    if let Some(handled) = vocal::try_handle(app, request) {
        return handled;
    }

    // Offline render (todo #1157): `render.mixdown` bounces the master
    // mix to a WAV file at an explicit path, as a job. Mutating dispatch
    // (it drives an engine command) — the task must reach the runtime.
    if let Some(handled) = render::try_handle(app, conn, request) {
        return handled;
    }

    if is_protocol_method(method) {
        // Known in protocol v1, but its namespace todo hasn't landed
        // yet. Stable `unsupported` kind either way; the message tells a
        // capable client the difference.
        return (
            failure(
                request,
                RpcError::unsupported(format!("method not implemented yet: {method}")),
            ),
            Task::none(),
        );
    }

    (
        failure(request, RpcError::method_not_found(method)),
        Task::none(),
    )
}

/// `control.hello` — version handshake. Records the declared version on
/// the connection's session; an incompatible version marks the session
/// so every later request is rejected.
fn hello(app: &mut Resonance, conn: ConnId, request: &Request) -> Response {
    let params: HelloParams = match request.params() {
        Ok(params) => params,
        Err(error) => return failure(request, error),
    };

    // Tests may drive `execute` without a preceding `Connected` event;
    // materialize the session either way.
    let session = app.control.sessions.entry(conn).or_default();
    session.protocol_version = Some(params.protocol_version);

    if params.protocol_version != PROTOCOL_VERSION {
        session.incompatible = true;
        return failure(
            request,
            RpcError::unsupported(format!(
                "incompatible protocol version {} (server speaks {})",
                params.protocol_version, PROTOCOL_VERSION
            )),
        );
    }

    session.incompatible = false;
    let result = HelloResult {
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        protocol_version: PROTOCOL_VERSION,
        capabilities: resonance_control::methods::capabilities()
            .iter()
            .map(|m| (*m).to_owned())
            .collect(),
    };
    success(request, &result)
}

/// True when `method` is part of protocol v1 (i.e. listed in the
/// `control.hello` capabilities).
fn is_protocol_method(method: &str) -> bool {
    use std::sync::OnceLock;
    static CAPABILITIES: OnceLock<Vec<&'static str>> = OnceLock::new();
    CAPABILITIES
        .get_or_init(resonance_control::methods::capabilities)
        .contains(&method)
}

// ---------------------------------------------------------------------------
// Shared helpers for the namespace handlers (todos #1148+)
// ---------------------------------------------------------------------------

/// Route a synthesized domain message through the FULL update path —
/// gates, frozen-input classifier, undo recording, dispatch, transaction
/// commit. This is the only way a mutating control method may touch
/// state; the returned [`Task`] must be handed back to the runtime (the
/// control handler returns it, `update.rs` forwards it).
pub(crate) fn run_via_update(app: &mut Resonance, message: Message) -> Task<Message> {
    app.update(message)
}

/// The `{revision}` acknowledgement every mutating reply carries,
/// snapshotting the app's monotonic undoable-transaction counter.
pub(crate) fn mutation_ack(app: &Resonance) -> resonance_control::MutationAck {
    resonance_control::MutationAck {
        revision: app.revision(),
    }
}

/// Why a mutating control method cannot run right now, or `None` when
/// the app can take the edit. Mirrors the pre-dispatch gates that would
/// otherwise silently swallow a synthesized domain message (startup
/// modal / offline bounce / freeze render) — a remote client must see a
/// stable `busy` error instead of a no-op that claims success.
pub(crate) fn mutation_gate_error(app: &Resonance) -> Option<RpcError> {
    if !app.io.has_active_project {
        return Some(RpcError::busy(
            "no active project — open or create one first",
        ));
    }
    if app.bounce_in_progress.is_some() {
        return Some(RpcError::busy(
            "an offline bounce is rendering; retry when it finishes",
        ));
    }
    if app.freeze.any_in_flight() {
        return Some(RpcError::busy(
            "a track freeze is rendering; retry when it finishes",
        ));
    }
    None
}

/// A success reply for `request`; falls back to an internal error if the
/// result fails to serialize (unreachable for well-formed result types).
fn success<T: serde::Serialize>(request: &Request, result: &T) -> Response {
    Response::success(request.id.clone(), result).unwrap_or_else(|e| {
        Response::failure(
            Some(request.id.clone()),
            RpcError::internal(format!("failed to encode result: {e}")),
        )
    })
}

/// An error reply for `request`.
fn failure(request: &Request, error: RpcError) -> Response {
    Response::failure(Some(request.id.clone()), error)
}
