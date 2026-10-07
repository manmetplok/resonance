//! `control.*` — connection handshake.

use serde::{Deserialize, Serialize};

/// `control.hello` — first call on a connection. Never mutates. The
/// server rejects clients that declare an incompatible major version.
pub const HELLO: &str = "control.hello";

/// All `control.*` method names.
pub const METHODS: &[&str] = &[HELLO];

/// Params for `control.hello`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct HelloParams {
    /// The client's [`crate::PROTOCOL_VERSION`].
    pub protocol_version: u32,
}

/// Result of `control.hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct HelloResult {
    /// The app's own version string (crate version).
    pub app_version: String,
    /// The server's [`crate::PROTOCOL_VERSION`].
    pub protocol_version: u32,
    /// Method names this server implements (see
    /// [`crate::methods::capabilities`]).
    pub capabilities: Vec<String>,
    /// The ONNX Runtime backend the vocal synth's acoustic model really
    /// runs on (`cpu`, `cuda`, `migraphx`), after any fallback to CPU.
    /// Absent until a vocal render has created a session in this app
    /// process. Optional, so older clients ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub svs_backend: Option<String>,
}
