//! Control-endpoint state (ba doc #265, todo #1147).
//!
//! GUI-side bookkeeping for the unix-socket control endpoint: the
//! listener lifecycle handle plus per-connection handshake state. The
//! socket threads themselves live in [`crate::control_socket`]; the
//! request handlers in `update/control`.

use crate::control_jobs::JobBoard;
use crate::control_socket::{ConnId, ControlServer};
use std::collections::HashMap;
use std::sync::Arc;

/// Per-connection session state, created on `Connected` and dropped on
/// `Disconnected`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ControlSession {
    /// The protocol version the client declared in `control.hello`.
    /// `None` until the client has said hello (allowed but discouraged —
    /// doc #265 says hello *should* be first).
    pub protocol_version: Option<u32>,
    /// Set when the client declared an incompatible protocol version.
    /// Every subsequent request on this connection is rejected.
    pub incompatible: bool,
}

/// The control endpoint's app-side state.
///
/// Transient: never persisted, never in the undo snapshot. The connected
/// client count feeds the window chrome's remote-control indicator
/// (todo #1159).
#[derive(Default)]
pub struct ControlEndpointState {
    /// Listener lifecycle handle. `None` when the endpoint is disabled
    /// (`RESONANCE_NO_CONTROL=1`), failed to bind, or was never started
    /// (tests construct the app without a socket).
    pub server: Option<ControlServer>,
    /// Live client connections keyed by connection id.
    pub sessions: HashMap<ConnId, ControlSession>,
    /// Async job ledger (todo #1149), shared with the socket threads:
    /// the update loop starts and resolves jobs, the per-connection
    /// reader threads block on it to serve `job.wait`.
    pub jobs: Arc<JobBoard>,
}
