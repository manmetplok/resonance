//! The MCP tool surface, one module per control-protocol area.
//!
//! Every module holds one `#[tool_router]` impl block on
//! [`crate::server::ResonanceMcp`] contributing a named router;
//! `ResonanceMcp::combined_router` sums them. Tool names are snake_case
//! and mirror the control methods 1:1 (`transport.seek` ->
//! `transport_seek`), so Claude Code exposes them as
//! `mcp__resonance__transport_seek`. Param/result schemas come from the
//! `resonance-control` wire types (schemars feature) — they cannot
//! drift from the protocol.

pub mod amp_models;
pub mod presets;
pub mod arrange;
pub mod automation;
pub mod bus;
pub mod clip;
pub mod compose;
pub mod control;
pub mod drum_kits;
pub mod edit;
pub mod external;
pub mod global;
pub mod master;
pub mod meter;
pub mod project;
pub mod render;
pub mod song;
pub mod trackmix;
pub mod transport;
pub mod vocal;
