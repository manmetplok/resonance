//! `job.*` handlers on the update loop (ba doc #265, todo #1149).
//!
//! `job.status` reads the shared [`JobBoard`](crate::control_jobs::JobBoard)
//! and never mutates project state. `job.wait` normally never reaches
//! this handler — the socket reader threads intercept it and block on
//! the board's condvar (`control_socket::job_wait_response`) so the
//! update loop can't stall; the arm here is the non-blocking fallback
//! for direct dispatch (tests, future transports): it answers with the
//! then-current status immediately, i.e. a zero-timeout wait.

use crate::Resonance;
use resonance_audio::types::ExportErrorKind;
use resonance_control::job::{self, StatusParams, WaitParams};
use resonance_control::{ErrorKind, Request, Response, RpcError};

/// Map an offline export/bounce failure's [`ExportErrorKind`] onto the
/// control protocol's [`ErrorKind`] (ARCH-05 / epic C, C-2), so
/// `JobStatus.error.kind` gives a `render.mixdown` job caller something
/// to branch on. Lives here, not in `resonance-audio`: the engine sits
/// below `resonance-control` in the crate DAG (see ARCHITECTURE.md) and
/// must not depend on it, so this is a hand-kept mirror rather than a
/// shared type — the same reasoning `EngineErrorKind`'s own doc comment
/// gives for mirroring `resonance_control::ErrorKind` instead of using it
/// directly.
///
/// `Cancelled` and `NoAudio` have no control-side analog (a cancel is not
/// really a client-actionable failure, and "nothing to render" is a
/// project-state problem, not a bad request); both fall back to
/// `Internal` rather than a kind that would overclaim precision.
pub(crate) fn export_kind_to_rpc(kind: ExportErrorKind) -> ErrorKind {
    match kind {
        ExportErrorKind::EncoderUnavailable => ErrorKind::Unsupported,
        ExportErrorKind::TransportRunning => ErrorKind::Busy,
        ExportErrorKind::Io | ExportErrorKind::Cancelled | ExportErrorKind::NoAudio => {
            ErrorKind::Internal
        }
    }
}

/// Handle a `job.*` request, or `None` when `method` belongs to another
/// namespace.
pub(super) fn try_handle(app: &Resonance, request: &Request) -> Option<Response> {
    let response = match request.method.as_str() {
        job::STATUS => status(app, request),
        job::WAIT => wait_snapshot(app, request),
        _ => return None,
    };
    Some(response)
}

fn status(app: &Resonance, request: &Request) -> Response {
    let params: StatusParams = match request.params() {
        Ok(p) => p,
        Err(e) => return super::failure(request, e),
    };
    match app.control.jobs.status(params.job_id.0) {
        Some(status) => super::success(request, &status),
        None => super::failure(
            request,
            RpcError::not_found(format!("no job with id {}", params.job_id)),
        ),
    }
}

/// The update-loop `job.wait` fallback: an immediate status snapshot
/// (never blocks — blocking waits are served on the socket threads).
fn wait_snapshot(app: &Resonance, request: &Request) -> Response {
    let params: WaitParams = match request.params() {
        Ok(p) => p,
        Err(e) => return super::failure(request, e),
    };
    match app.control.jobs.status(params.job_id.0) {
        Some(status) => super::success(request, &status),
        None => super::failure(
            request,
            RpcError::not_found(format!("no job with id {}", params.job_id)),
        ),
    }
}
