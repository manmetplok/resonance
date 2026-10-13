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
        description = "Bounce the master mix to a WAV file at an absolute path. Replacing an \
                       existing file needs overwrite: true. Runs as a job — waits up to 5 \
                       minutes and returns the final status whose result carries {path, \
                       duration_s, sample_rate, soloed_track_ids}. A non-empty \
                       soloed_track_ids means the file is NOT the mix — only those tracks are \
                       in it; check it before calling a bounce a master. The transport must be stopped (a running or \
                       recording transport is refused as busy), and only one render at a time. \
                       \
                       RANGE: range {start, end} (song positions, e.g. {start: {bar: 9}, end: \
                       {bar: 17}}) renders just that window — sample 0 of the file is the range \
                       start — so a chorus can be auditioned without a full-length render. A \
                       range past the song's end is clamped to it; an empty one is refused. \
                       Without a range the file spans the union of ALL clips in the project: \
                       sample 0 of the WAV is the EARLIEST clip's start, which is not bar 1 \
                       unless something starts there. Either way 2 s of FX tail follow the end. \
                       Do not convert file offsets into bar positions — read positions from \
                       song_tracks (clip start/length) and song_notes. See mixer_set_solo for \
                       why a soloed bounce is an especially bad ruler. \
                       \
                       DELIVERY: normalize: {target_lufs, ceiling_dbtp} writes a \
                       loudness-normalized file: the mix is measured, gained to target_lufs \
                       integrated (-40..-5), then true-peak limited at ceiling_dbtp (-12..0). \
                       Or pass platform instead, shorthand for a published target: spotify, \
                       youtube and tidal -14 LUFS / -1 dBTP; apple -16 / -1; amazon -14 / -2; \
                       deezer -15 / -1; club -8 / -0.3. Pass one of the two, not both. The \
                       project is untouched; only the file is normalized. The result then \
                       carries normalize {platform, target_lufs, ceiling_dbtp, achieved_lufs, \
                       achieved_dbtp}, re-measured from the written file: achieved_lufs can \
                       sit below the target when reaching it would take more limiting than the \
                       ceiling allows, which is a sign the master needs its own limiting \
                       before delivery rather than more gain here.",
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
        description = "Export per-track stems: one WAV per top-level track into the directory \
                       dir (absolute; created if missing), named NN-<track name>.wav in track-id \
                       order exactly as the app's Export dialog names them, 48 kHz 24-bit, every \
                       stem over the same range so they line up when re-imported. A track's stem \
                       is that track alone after its own fader, pan and effects (its sub-tracks \
                       included), without the master chain. track_ids: [...] exports only those \
                       tracks — e.g. just the vocal, for a vocal-only file without soloing; an \
                       unknown id or a sub-track is refused as invalid_params. range works as in \
                       render_mixdown (sample 0 = the range start; omitted = every clip, sample 0 \
                       the earliest one), plus 2 s of FX tail. A stem file already in dir under \
                       the same name is refused as needs_confirmation until you pass overwrite: \
                       true. Runs as a job — waits up to 5 minutes and returns the final status \
                       whose result carries {paths, duration_s}. One render at a time; the \
                       transport must be stopped. If any stem fails the job fails, its message \
                       listing the errors and the files that were written.",
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
                       progress 0..1; result carries the job's payload once done. On error, \
                       error.message is always present and error.kind is set when the failure \
                       was classified (e.g. not_found, busy, unsupported, internal) — branch on \
                       kind rather than parsing message. Read-only. job_ids come from project_*, \
                       vocal_render and render_* results.",
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
                       then-current status either way). On error, error.message is always \
                       present and error.kind is set when the failure was classified (e.g. \
                       not_found, busy, unsupported, internal) — branch on kind rather than \
                       parsing message. Waits at most 10 minutes: omitting timeout_ms, or \
                       passing more than 600000, waits the full 10 minutes. Other tool calls \
                       keep working while it waits.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn job_wait(
        &self,
        Parameters(params): Parameters<WaitParams>,
    ) -> Result<CallToolResult, McpError> {
        self.wait_for_job(params.job_id, params.timeout_ms).await
    }
}
