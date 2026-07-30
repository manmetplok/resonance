//! `vocal_*` — lyrics, pronunciation overrides, and SVS rendering.
//!
//! Every lane-addressed tool here needs a vocal lane on the track first;
//! `section_set_lane_generator` with kind `vocal` is what creates one
//! (ba doc #268).

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::job::JobStatus;
use resonance_control::methods::vocal;

/// SVS rendering is heavy (neural synthesis); wait longer than for
/// project I/O before handing back the job id.
const VOCAL_RENDER_WAIT_MS: u64 = 120_000;

#[tool_router(router = router_vocal, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Replace a vocal track's full lyric text (one line per lyric line). \
                       Phonemes are derived automatically (G2P); check them with song_vocal \
                       and fix words with vocal_set_pronunciation. Re-render afterwards with \
                       vocal_render. Lyrics live per (section, track) vocal lane, created \
                       with section_set_lane_generator kind \"vocal\": pass section_id (a \
                       definition_id from song_vocal's lanes) to pick one, or omit it to write \
                       the track's FIRST vocal lane.",
        annotations(destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn vocal_set_lyrics(
        &self,
        Parameters(params): Parameters<vocal::SetLyricsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(vocal::SET_LYRICS, &params).await
    }

    #[tool(
        description = "Replace one lyric line, addressed by its 0-based line_index from \
                       song_vocal. Like vocal_set_lyrics, section_id picks which vocal lane \
                       (lyrics live per (section, track) lane); omitted it writes the track's \
                       FIRST lane.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn vocal_set_line(
        &self,
        Parameters(params): Parameters<vocal::SetLineParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(vocal::SET_LINE, &params).await
    }

    #[tool(
        description = "Override how a word is sung: lowercase ARPAbet-style phonemes, e.g. \
                       word \"lilia\", phonemes [\"l\",\"ih\",\"l\",\"iy\",\"ah\"]. Project-wide \
                       and case-insensitive; applies on the next vocal_render.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn vocal_set_pronunciation(
        &self,
        Parameters(params): Parameters<vocal::SetPronunciationParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(vocal::SET_PRONUNCIATION, &params).await
    }

    #[tool(
        description = "Remove a per-word pronunciation override, reverting to automatic G2P.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn vocal_clear_pronunciation(
        &self,
        Parameters(params): Parameters<vocal::ClearPronunciationParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(vocal::CLEAR_PRONUNCIATION, &params).await
    }

    #[tool(
        description = "Render the singing voice (SVS). Omit track_id to render every vocal \
                       track; voicebank defaults to the app default (Lilia). Runs as a job — \
                       this tool waits up to 2 minutes and returns the final status; if still \
                       running, poll job_status with the returned job_id. Needs a vocal lane \
                       (section_set_lane_generator kind \"vocal\") with notes AND lyrics on the \
                       vocal track first.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn vocal_render(
        &self,
        Parameters(params): Parameters<vocal::RenderParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(vocal::RENDER, &params, VOCAL_RENDER_WAIT_MS)
            .await
    }
}
