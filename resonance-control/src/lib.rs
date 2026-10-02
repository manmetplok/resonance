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
//! removing methods/fields bumps [`PROTOCOL_VERSION`].
//!
//! **A change of meaning is a breaking change too.** When a field keeps
//! its name but is read differently (CTL-01 changed what `beat` in
//! [`PositionSpec`] counts in 6/8), bump [`PROTOCOL_VERSION`] or add a
//! capability the client checks for, so a peer built against the old
//! meaning refuses to talk instead of silently doing the wrong thing.
//!
//! **Requests are strict, results are tolerant.** Every request `*Params`
//! type, and the request-only specs nested in them (`PositionSpec`,
//! `AutomationTargetSpec`, `PresetFilter`, …), is
//! `#[serde(deny_unknown_fields)]`: a misspelled or wrong-surface field
//! (`beats` for `beat`) is `invalid_params` naming the field, never a
//! success that did nothing (code review ARCH2-06;
//! `tests/unknown_fields.rs` holds every `*Params` to it). This works
//! through `#[serde(flatten)]` because every flattened part is a plain
//! struct, which takes its own keys before the outer type checks what
//! is left; do not flatten an enum or a map into params. Result and view
//! types stay tolerant of unknown fields, so an older client still reads
//! a newer app's replies.
//!
//! **One exception, and it is deliberate: retiring a method that was
//! already published as deprecated does not bump the version.** A rename
//! keeps the old spelling reachable for a release and says so in the
//! constant's own doc (see the `track.plugins` -> `plugins.catalog`
//! rename in todos #1236/#1240); dropping it at the end of that window
//! is the second half of a change the version was never asked to
//! describe, and the announcement was the deprecation, not the number.
//!
//! The reason this is worth writing down rather than judging case by
//! case: the handshake compares versions with `!=`, not `>=`, so a bump
//! refuses every client that has not been rebuilt — including the
//! installed `resonance-mcp` binary — and forces a matching re-pin of
//! the agent plugin's `lockstep.json`. Bumping to announce the removal
//! of a name with no MCP tool and no known caller would break far more
//! than it documents. A method that was NOT deprecated first still bumps;
//! that is the whole point of the window.

pub mod common;
pub mod framing;
pub mod ids;
pub mod job;
pub mod methods;
pub mod rpc;
pub mod socket;

pub use common::{
    check_max_bars, BeatRange, KeyScale, MutationAck, PositionSpec, SongPosition, TimeSignature, TrackKind,
    TrackOutput, TransportState, MAX_BARS,
};
pub use framing::{write_message, FramingError, MessageReader};
pub use job::{JobStarted, JobState, JobStatus};
pub use rpc::{ErrorData, ErrorKind, Request, RequestId, Response, RpcError};

/// Version of this control protocol. Bumped only on breaking changes;
/// additions (new methods, new optional fields) keep the same version.
pub const PROTOCOL_VERSION: u32 = 1;
