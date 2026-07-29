//! resonance-mcp — MCP stdio server for driving a running resonance app.
//!
//! Bridges the Model Context Protocol (stdio transport, [`rmcp`] SDK) onto
//! resonance's unix-socket control protocol (`resonance-control`, ba doc
//! #265): each MCP tool maps 1:1 onto a control method, with params and
//! results typed by the exact wire structs (schemars feature) so the
//! published tool schemas can never drift from the protocol.
//!
//! Register with Claude Code:
//! `claude mcp add --transport stdio resonance -- /path/to/resonance-mcp`.

pub mod client;
pub mod server;
pub mod tools;

pub use client::{socket_path, CallError, ControlClient};
pub use server::ResonanceMcp;
