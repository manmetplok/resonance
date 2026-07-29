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
use resonance_control::job::{self, StatusParams, WaitParams};
use resonance_control::{Request, Response, RpcError};

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
