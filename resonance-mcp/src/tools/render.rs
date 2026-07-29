//! `render_*` / `job_*` — offline audio export and job tracking.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::job::{self, JobStatus, StatusParams, WaitParams};
use resonance_control::methods::render;

/// Offline bounces render faster than realtime but scale with song
/// length; wait generously before handing back the job id.
const RENDER_WAIT_MS: u64 = 300_000;

#[tool_router(router = router_render, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Bounce the master mix to a WAV file at an absolute path; optional range \
                       (start/end as bar[+beat] or sample) defaults to the whole song. \
                       Replacing an existing file needs overwrite: true. Runs as a job — waits \
                       up to 5 minutes and returns the final status whose result carries \
                       {path, duration_s}.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn render_mixdown(
        &self,
        Parameters(params): Parameters<render::MixdownParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(render::MIXDOWN, &params, RENDER_WAIT_MS)
            .await
    }

    #[tool(
        description = "Export per-track stem WAVs into an absolute directory; optional range \
                       defaults to the whole song. Replacing existing files needs overwrite: \
                       true. Runs as a job — waits up to 5 minutes and returns the final \
                       status whose result carries {paths, duration_s}.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn render_stems(
        &self,
        Parameters(params): Parameters<render::StemsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(render::STEMS, &params, RENDER_WAIT_MS).await
    }

    #[tool(
        description = "Current state of a job (pending | running | done | error) with optional \
                       progress 0..1; result carries the job's payload once done. Read-only. \
                       job_ids come from project_*, vocal_render and render_* results.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn job_status(
        &self,
        Parameters(params): Parameters<StatusParams>,
    ) -> Result<CallToolResult, McpError> {
        self.job_query(job::STATUS, params.job_id, &params).await
    }

    #[tool(
        description = "Block until a job reaches done/error or timeout_ms elapses (returns the \
                       then-current status either way; omit timeout_ms to wait indefinitely). \
                       Prefer a bounded timeout so the conversation stays responsive.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn job_wait(
        &self,
        Parameters(params): Parameters<WaitParams>,
    ) -> Result<CallToolResult, McpError> {
        self.job_query(job::WAIT, params.job_id, &params).await
    }
}
