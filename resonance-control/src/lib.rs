//! Versioned control protocol for driving a running resonance app from an
//! external client (the `resonance-mcp` MCP server, tests, scripts).
//!
//! This crate is the single source of truth both sides compile against
//! (design: ba doc #265). It deliberately has **no** app/audio/iced
//! dependencies — only serde types plus line-framing helpers:
//!
//! - [`rpc`]: the JSON-RPC 2.0 envelope ([`Request`], [`Response`],
//!   [`RpcError`]) with the stable machine-readable error kinds.
//! - [`framing`]: newline-delimited JSON framing over any
//!   `Read`/`Write` pair (one UTF-8 JSON message per `\n`-terminated line).
//! - [`methods`]: per-namespace param/result types (`control.*`, `song.*`,
//!   `project.*`, `transport.*`, `track.*`, `mixer.*`, `section.*`,
//!   `harmony.*`, `generate.*`, `notes.*`, `vocal.*`, `render.*`).
//! - [`job`]: async job tracking (`job.status` / `job.wait`) for
//!   long-running operations (project I/O, SVS render, mixdown).
//! - [`ids`] / [`common`]: shared id newtypes (the app's real `u64` ids,
//!   serialized verbatim) and compact view primitives.
//! - [`socket`]: the socket-path resolution rule, so server and client
//!   can never disagree on where the socket lives.
//!
//! Protocol evolution is additive within a major version; renaming or
//! removing methods/fields bumps [`PROTOCOL_VERSION`]. Deserialization is
//! tolerant of unknown fields so newer peers can talk to older ones.

pub mod common;
pub mod framing;
pub mod ids;
pub mod job;
pub mod methods;
pub mod rpc;
pub mod socket;

pub use common::{
    BeatRange, KeyScale, MutationAck, PositionSpec, SongPosition, TimeSignature, TrackKind,
    TrackOutput, TransportState,
};
pub use framing::{write_message, FramingError, MessageReader};
pub use job::{JobStarted, JobState, JobStatus};
pub use rpc::{ErrorData, ErrorKind, Request, RequestId, Response, RpcError};

/// Version of this control protocol. Bumped only on breaking changes;
/// additions (new methods, new optional fields) keep the same version.
pub const PROTOCOL_VERSION: u32 = 1;
