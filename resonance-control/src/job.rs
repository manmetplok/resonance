//! Async job tracking (`job.*`).
//!
//! Methods that complete asynchronously in the app (project save/load,
//! SVS vocal render, WAV/stem export) return [`JobStarted`] immediately;
//! clients poll `job.status` or block on `job.wait` (which blocks the
//! socket thread, never the app's update loop).

use crate::ids::JobId;
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
/// `done` (e.g. [`crate::methods::render::MixdownResult`]); `error` is a
/// human-readable failure message once `state` is `error`.
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
    pub error: Option<String>,
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
