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

mod arrangement;
mod bus;
mod chain_presets;
mod clip;
mod edit;
/// The slot-or-(plugin_id, occurrence) effect-addressing state machine,
/// shared by `track/bus/master.remove_effect` / `move_effect` /
/// `replace_effect` so the three cannot silently disagree about what a
/// legal address is (ba doc #275).
mod effect_addressing;
mod external;
mod generate;
mod global;
mod harmony;
mod import_midi;
mod job;
mod master;
mod meter;
mod notes;
/// Per-slot and whole-chain bypass, shared by all three surfaces
/// (ba todo #1305).
mod bypass;
mod plugin_presets;
/// `plugins.rescan` — the installed-plugin catalog's one mutation
/// (todo #1307). `plugins.catalog` is read-only and lives in `song`.
mod plugins;
mod project;
mod render;
/// The shared half of `track/bus/master.replace_effect` (todo #1309).
mod replace;
/// The reply vocabulary every namespace answers with (todo #1258).
mod reply;
mod section;
mod sidechain;
mod song;
mod track;
mod transport;
/// App state -> wire projection, shared by every namespace (todo #1256).
mod view_model;
mod vocal;

/// Re-exported so every namespace module keeps reaching them as
/// `super::success` / `super::failure` — one implementation, in
/// [`reply`], for the whole layer.
use reply::{failure, success};

pub(crate) use clip::{import_result, place_result};
pub(crate) use job::export_kind_to_rpc;
pub(crate) use plugin_presets::write_saved_state as write_plugin_preset;
pub(crate) use meter::{mix_measure_error, mix_measured};
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
            // Drop the connection's terminal jobs and orphan its live
            // ones: the operations behind live jobs keep running, and
            // the MCP client reconnects after any transport failure
            // expecting `job.status` on the same (global) job id to
            // still find them. A reader blocked in `job.wait` on a
            // dropped terminal job resolves to `not_found`.
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

    // Mutation gate (todo #1161): every namespace below this line
    // mutates project state, so it needs an active, non-busy project.
    // Enforcing it here — once, structurally — instead of inside each
    // handler means a newly added mutating namespace cannot silently
    // no-op and claim success (the bug the per-handler convention kept
    // reintroducing). The read-only views (`song.*`, `job.*`,
    // `control.hello`) and the project-lifecycle methods (`project.*`,
    // which create/open the very project this gate checks for) ran
    // *above* and are intentionally not gated.
    //
    // Restricted to *known* protocol methods so an unknown method still
    // falls through to `method_not_found` rather than being masked by a
    // `busy` — the gate answers "can't right now", not "no such method".
    if is_protocol_method(method) && !is_read_only_method(method) {
        if let Some(error) = mutation_gate_error(app, method) {
            return (failure(request, error), Task::none());
        }
        // One call, one undo entry, one revision bump — however many
        // messages the handler dispatches, and never coalesced with the
        // previous call on the same control (code review CTL-03).
        return app.with_compound_undo(|app| execute_mutating(app, conn, request));
    }
    execute_mutating(app, conn, request)
}

/// The mutating namespaces of [`execute`], below the mutation gate.
fn execute_mutating(
    app: &mut Resonance,
    conn: ConnId,
    request: &Request,
) -> (Response, Task<Message>) {
    let method = request.method.as_str();

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
    if let Some(handled) = vocal::try_handle(app, conn, request) {
        return handled;
    }

    // Offline render (todo #1157): `render.mixdown` bounces the master
    // mix to a WAV file at an explicit path, as a job. Mutating dispatch
    // (it drives an engine command) — the task must reach the runtime.
    if let Some(handled) = render::try_handle(app, conn, request) {
        return handled;
    }

    // Mix measurement (todo #1219): renders a slice of the mix offline
    // and reports its BS.1770 numbers, as a job. Read-only in the sense
    // that it changes nothing — but it describes the OPEN project, so
    // like `master.summary` it sits below the gate and is absent from
    // `is_read_only_method`: with nothing open the honest answer is
    // `busy`, not a measurement of silence.
    if let Some(handled) = meter::try_handle(app, conn, request) {
        return handled;
    }

    // Mutating transport namespace (todo #1150): synthesizes the
    // existing TransportMessage variants through the full update path.
    if let Some(result) = transport::try_handle(app, request) {
        return result;
    }

    // Mutating track/mixer namespace (todo #1152): add/rename/delete
    // tracks, volume/pan/mute/solo, add built-in instrument/FX.
    if let Some(result) = track::try_handle(app, request) {
        return result;
    }

    // `plugins.rescan` (todo #1307): refresh the installed-plugin
    // catalog without restarting. Mutating (it drives an engine
    // command), but project-independent — it ran above the gate for the
    // same reason `plugins.catalog` does.
    if let Some(result) = plugins::try_handle(app, request) {
        return result;
    }

    // External instruments: the MIDI-out + audio-return route, patch,
    // latency, monitoring and the realtime capture. `external.devices`
    // and `external.status` read only, but they sit below the gate on
    // purpose — they describe the OPEN project's hardware wiring, so
    // with nothing open the honest answer is `busy`, not an empty list.
    if let Some(result) = external::try_handle(app, request) {
        return result;
    }

    // Master bus (todo #1226): the final summing stage. `master.summary`
    // reads only, but it sits below the gate on purpose — it describes
    // the open project's master, so with no project the honest answer is
    // `busy`, not a default.
    if let Some(result) = master::try_handle(app, request) {
        return result;
    }

    // Group busses (todo #1228): create/delete/level. Listing lives in
    // `song.*`, which already reports busses as `kind: "bus"`.
    if let Some(result) = bus::try_handle(app, request) {
        return result;
    }

    // Undo / redo (todo #1196): pops the app's EXISTING history. Like
    // `master.summary`, `edit.status` reads only but describes the open
    // project, so it sits below the gate.
    if let Some(result) = edit::try_handle(app, request) {
        return result;
    }

    // Piano-roll note editing (todo #1155): notes.insert/edit/delete +
    // create_clip, synthesizing MidiEditor / MidiClip messages.
    if let Some(result) = notes::try_handle(app, request) {
        return result;
    }

    // Samples: the media pool (`pool.*`) and audio-clip placement/editing
    // (`clip.*`). `pool.list` reads only, but like `master.summary` it
    // describes the OPEN project, so it sits below the gate — with nothing
    // open the honest answer is `busy`, not an empty pool.
    if let Some(result) = clip::try_handle(app, conn, request) {
        return result;
    }

    // Structural bar shifts (ba doc #275 P2): the one edit that has to
    // move every timeline collection at once, which is why it cannot be
    // assembled out of the per-object methods above.
    if let Some(result) = arrangement::try_handle(app, request) {
        return result;
    }

    // The global tempo / time-signature tracks (ba doc #286). Sits below
    // the gate like `master.summary`: `global.list_events` mutates
    // nothing, but it reports the OPEN project's tempo map, so with
    // nothing open `busy` is honest and a default 120 BPM 4/4 is not.
    if let Some(result) = global::try_handle(app, request) {
        return result;
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

/// True when `method` needs no active project and so bypasses
/// [`mutation_gate_error`] (todo #1161): the read-only introspection
/// (`control.hello`, `song.*`, `job.*`) plus the project-lifecycle
/// methods (`project.*`), which are the ones that *establish* the
/// project the gate otherwise requires. Every other method mutates the
/// current project and is gated.
///
/// `plugins.*` is the one addition to that list: it is read-only AND
/// project-independent (unlike `master.summary` / `edit.status`, which
/// read but describe the open project and so stay gated).
///
/// The `notes.*` create/insert/etc., `transport.*`, `track.*` /
/// `mixer.*`, `section.*`, `harmony.*`, `generate.*`, `vocal.*` and
/// `render.*` namespaces are all mutating and deliberately absent. So is
/// `meter.*` (todo #1219), which mutates nothing but measures the open
/// project — a stable `busy` with nothing open beats a measurement of a
/// project that isn't there.
pub(crate) fn is_read_only_method(method: &str) -> bool {
    use resonance_control::methods;
    method == HELLO
        || methods::song::METHODS.contains(&method)
        || methods::project::METHODS.contains(&method)
        // `plugins.catalog` reads `available_plugins`, which the
        // scanner fills at startup
        // and which no project owns. Gating it made a pure catalog query
        // answer `busy` with nothing open, so an agent could not even
        // find out what it had to build with before opening a project
        // (todo #1236).
        || methods::plugins::METHODS.contains(&method)
        || resonance_control::job::METHODS.contains(&method)
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

/// Validate a time signature exactly as the app's own signature track
/// does: numerator `1..=32`, denominator a power of two in `1..=32`
/// (resolved, e.g. `8` for 7/8 — never the exponent).
///
/// One definition for the whole control layer: `transport.set_time_signature`
/// rewrites the bar-1 event and `global.*` writes the rest of the
/// signature track, and the two must not be able to disagree about what
/// a legal meter is (ba doc #286 §2). Rejects rather than clamps, so a
/// client learns why instead of silently getting a different meter.
pub(super) fn validate_time_signature(numerator: u8, denominator: u8) -> Result<(), RpcError> {
    if numerator == 0 || numerator > 32 {
        return Err(RpcError::invalid_params(format!(
            "time-signature numerator {numerator} out of range 1..=32"
        )));
    }
    if !matches!(denominator, 1 | 2 | 4 | 8 | 16 | 32) {
        return Err(RpcError::invalid_params(format!(
            "time-signature denominator {denominator} must be a power of two (1..=32)"
        )));
    }
    Ok(())
}

/// Validate a tempo exactly as the app's own tempo field does: finite,
/// and inside [`BPM_RANGE`].
///
/// One definition for the whole control layer, for the same reason as
/// [`validate_time_signature`]: `transport.set_tempo` writes the bar-1
/// tempo event and `global.add_tempo_event` writes the rest of the tempo
/// track, and the two must not be able to disagree about what a legal
/// tempo is. Rejects rather than clamps (which is what the GUI's
/// `CommitBpm` / `UpdateTempoEvent` do), so a client learns why instead
/// of silently getting a different tempo than it asked for.
pub(super) fn validate_bpm(bpm: f64) -> Result<(), RpcError> {
    if !bpm.is_finite() || !BPM_RANGE.contains(&bpm) {
        return Err(RpcError::invalid_params(format!(
            "bpm {bpm} out of range {BPM_RANGE:?}"
        )));
    }
    Ok(())
}

/// Tempo range every control method accepts, mirroring the clamp in
/// `CommitBpm` and `GlobalTrackMessage::UpdateTempoEvent`.
const BPM_RANGE: std::ops::RangeInclusive<f64> = 20.0..=300.0;

/// Parse params for a method whose params are entirely optional:
/// absent/null params mean "defaults". (`Request::params` alone maps
/// absent to JSON `null`, which serde refuses to turn into a struct.)
pub(super) fn optional_params<T: serde::de::DeserializeOwned + Default>(
    request: &Request,
) -> Result<T, RpcError> {
    match &request.params {
        None | Some(serde_json::Value::Null) => Ok(T::default()),
        Some(_) => request.params(),
    }
}

/// Why a mutating control method cannot run right now, or `None` when
/// the app can take the edit. Mirrors the pre-dispatch gates that would
/// otherwise silently swallow a synthesized domain message (startup
/// modal / offline bounce / freeze render) — a remote client must see a
/// stable `busy` error instead of a no-op that claims success.
pub(crate) fn mutation_gate_error(app: &Resonance, method: &str) -> Option<RpcError> {
    if !app.io.has_active_project {
        return Some(RpcError::busy(
            "no active project — open or create one first",
        ));
    }
    // Between `ClearAll` and `AllCleared` (a project load, a template
    // instantiation, a structural undo/redo) the replay is about to
    // rebuild everything from its snapshot: an edit taken now would be
    // acknowledged and then wiped (code review UPD-03).
    if app.io.loading || app.io.pending_load.is_some() {
        return Some(RpcError::busy(
            "a project load / undo replay is in progress; retry shortly",
        ));
    }
    // A WAV / FLAC mixdown (GUI bounce or `render.mixdown`) drives the
    // live plugin instances like a bounce in place, and the GUI gate
    // drops every synthesized domain message while it runs (code review
    // UPD-06) — so a mutation must answer `busy` here rather than no-op.
    // `meter.*` is the one namespace that stays open: its own render
    // guard refuses the offline source and keeps the live master meter
    // readable during a render, which is exactly when a client wants it.
    if app.io.bouncing && !resonance_control::methods::meter::METHODS.contains(&method) {
        return Some(RpcError::busy(
            "an offline render is in progress; retry when it finishes",
        ));
    }
    offline_render_busy_error(app)
}

/// `busy` while an offline bounce or a track freeze holds the offline
/// renderer. Split out of [`mutation_gate_error`] because the
/// destructive project-lifecycle methods (`project.new` / `project.open`)
/// run *above* the mutation gate — they must establish a project the
/// gate otherwise requires — yet still must not swap the project out
/// from under an in-flight render. They reuse this exact check (and its
/// wording) so both surfaces refuse identically.
pub(crate) fn offline_render_busy_error(app: &Resonance) -> Option<RpcError> {
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
