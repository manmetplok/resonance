//! `control.*` — connection handshake.

use serde::{Deserialize, Serialize};

/// `control.hello` — first call on a connection. Never mutates. The
/// server rejects clients that declare an incompatible major version.
pub const HELLO: &str = "control.hello";

/// All `control.*` method names.
pub const METHODS: &[&str] = &[HELLO];

/// Params for `control.hello`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloParams {
    /// The client's [`crate::PROTOCOL_VERSION`].
    pub protocol_version: u32,
}

/// Result of `control.hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloResult {
    /// The app's own version string (crate version).
    pub app_version: String,
    /// The server's [`crate::PROTOCOL_VERSION`].
    pub protocol_version: u32,
    /// Method names this server implements (see
    /// [`crate::methods::capabilities`]).
    pub capabilities: Vec<String>,
}
