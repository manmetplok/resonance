//! Async job tracking (`job.*`).
//!
//! Methods that complete asynchronously in the app (project save/load,
//! SVS vocal render, WAV/stem export) return [`JobStarted`] immediately;
//! clients poll `job.status` or block on `job.wait` (which blocks the
//! socket thread, never the app's update loop).

use crate::ids::JobId;
use crate::rpc::ErrorKind;
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `job.status` — read a job's current state. Never mutates.
pub const STATUS: &str = "job.status";
/// `job.wait` — block until the job reaches a terminal state or the
/// timeout elapses (returns the then-current status either way).
pub const WAIT: &str = "job.wait";

/// All `job.*` method names.
pub const METHODS: &[&str] = &[STATUS, WAIT];

/// Job lifecycle states, lowercase on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Pending,
    Running,
    Done,
    Error,
}

impl JobState {
    /// True for `done` and `error` — the states `job.wait` resolves on.
    pub fn is_terminal(&self) -> bool {
        matches!(self, JobState::Done | JobState::Error)
    }
}

/// Immediate result of any job-launching method: `{"job_id":7}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct JobStarted {
    pub job_id: JobId,
}

/// Result of `job.status` / `job.wait`.
///
/// `result` carries the job's method-specific payload once `state` is
/// `done` (e.g. [`crate::methods::render::MixdownResult`]); `error`
/// carries the failure once `state` is `error`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct JobStatus {
    pub job_id: JobId,
    pub state: JobState,
    /// Fractional progress in `0.0..=1.0` where the operation reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schemars", schemars(schema_with = "any_json_value_schema"))]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
}

/// A `job.status` failure (ARCH-05 / epic C, C-2). `message` is the
/// human-readable text this field always carried; `kind` is new — the
/// same `{kind, message}` shape [`crate::rpc::ErrorData`] uses for a
/// synchronous RPC failure and `resonance_audio::types::EngineError`
/// uses for an engine failure — so a control client branches on it
/// instead of pattern-matching `message`.
///
/// `kind` is absent when the underlying engine failure carries no typed
/// classification yet (a `Result<_, String>` site `thiserror` hasn't
/// reached), or when the job failed before any engine round-trip (a
/// plain `JobBoard::fail` with no kind supplied).
///
/// Serializes as `{"message": "...", "kind": "not_found"}` (or without
/// `kind` when absent). Deserializes that shape *or* a bare JSON string
/// — the pre-C-2 wire shape of this field — so an older peer's
/// `"error": "some message"` still parses under the current type.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct JobError {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ErrorKind>,
}

impl JobError {
    pub fn new(message: impl Into<String>, kind: Option<ErrorKind>) -> Self {
        JobError {
            message: message.into(),
            kind,
        }
    }
}

impl From<String> for JobError {
    fn from(message: String) -> Self {
        JobError { message, kind: None }
    }
}

impl<'de> Deserialize<'de> for JobError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            /// Pre-C-2 wire shape.
            Legacy(String),
            Typed {
                message: String,
                #[serde(default)]
                kind: Option<ErrorKind>,
            },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Legacy(message) => JobError { message, kind: None },
            Wire::Typed { message, kind } => JobError { message, kind },
        })
    }
}

/// Schema for [`JobStatus::result`]. schemars renders a bare
/// `serde_json::Value` as the JSON Schema boolean `true` ("any"), which
/// strict MCP clients (Claude Code / the Anthropic API) reject at
/// `outputSchema.properties.result`. Emit an equivalent *object* schema
/// ("any JSON, shape depends on the job kind") instead.
#[cfg(feature = "schemars")]
fn any_json_value_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "description": "Job-specific result payload once `state` is `done`; \
shape depends on the job kind (e.g. a mixdown/save path or a rendered clip id)."
    })
}

/// Params for `job.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct StatusParams {
    pub job_id: JobId,
}

/// Params for `job.wait`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct WaitParams {
    pub job_id: JobId,
    /// Maximum time to block, in ms. The app caps every wait at 600000
    /// (10 minutes); omitted means that cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}
