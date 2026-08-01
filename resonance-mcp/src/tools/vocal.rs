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
        description = "Generate a vocal lane: a melody into its MIDI clip and, unless \
                       lyrics: false, a fresh lyric draft from the lane's theme brief. This is \
                       what puts notes on a vocal lane — generate_part refuses vocal tracks, \
                       and vocal_render has nothing to sing without it. Needs a vocal lane \
                       (section_set_lane_generator kind \"vocal\") on a section that already \
                       has chords. section_id picks the lane; seed makes the result \
                       reproducible. Pass lyrics: false to keep lyrics you wrote with \
                       vocal_set_lyrics — the default regenerates them. Returns the clip_id to \
                       inspect with song_notes and edit with notes_*, then vocal_render.",
        annotations(destructive_hint = false, idempotent_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<vocal::GenerateResult>()
    )]
    async fn vocal_generate(
        &self,
        Parameters(params): Parameters<vocal::GenerateParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(vocal::GENERATE, &params).await
    }

    #[tool(
        description = "Replace a vocal track's full lyric text, one line per \\n-separated line. \
                       \
                       SVS sings ONE SYLLABLE PER NOTE, so the syllable count — not the word \
                       count — is what has to match the lane's notes. You may write breaks \
                       yourself with - or · (\"re-so-lu-tion\"); by default (syllabify: true) \
                       words you did not break are split for you, and your own breaks and [..] \
                       phoneme blocks are left alone. Pass syllabify: false only when you want \
                       the text stored exactly as typed, and then expect to place every break \
                       yourself — an unbroken multi-syllable word crams all its phonemes onto \
                       one note and is heard as a smear, not as the word. \
                       \
                       ALWAYS re-read song_vocal after writing. It is the cheapest verification \
                       available: syllable_count vs note_count with counts_mismatch, plus the \
                       per-note phoneme/duration budget that says which notes are too short to \
                       articulate what you gave them. A mismatch otherwise only surfaces as a \
                       bad render. Phonemes are derived automatically (G2P); fix individual \
                       words with vocal_set_pronunciation, then re-render with vocal_render. \
                       \
                       Lyrics live per (section, track) vocal lane, created with \
                       section_set_lane_generator kind \"vocal\": pass section_id (a \
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
        description = "Replace ONE lyric line, addressed by its 0-based line_index from \
                       song_vocal — the surgical alternative to rewriting the whole lane with \
                       vocal_set_lyrics. Same syllable rules: one syllable sings on one note, \
                       breaks may be written as - or ·, and syllabify (default true) splits the \
                       words you did not break. Like vocal_set_lyrics, section_id picks which \
                       vocal lane (lyrics live per (section, track) lane); omitted it writes the \
                       track's FIRST lane. Re-read song_vocal afterwards to confirm the lane's \
                       counts still line up.",
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
        description = "Render the singing voice (SVS): it synthesises the notes currently in \
                       the target lane's MIDI clip and never changes them, so author with \
                       vocal_generate + notes_* first and render last. Scope widens as you \
                       omit arguments: section_id renders exactly that lane; track_id alone \
                       renders EVERY lane on that track; omitting both renders every vocal \
                       lane in the project. Lanes render sequentially and the job only \
                       reports done once the last lane lands, so one call per track is \
                       enough — a per-lane loop is not needed. Lanes that cannot render (no \
                       notes or no lyrics) are skipped rather than failing the batch. \
                       voicebank is resolved per lane, so a track whose lanes chose \
                       different banks keeps them; it defaults to the app default (Lilia). \
                       Caution: song_vocal's render_state reports `rendered` when ANY lane \
                       on the track has audio, so it is not proof that every lane is \
                       current. Runs as a job — this tool waits up \
                       to 2 minutes and returns the final status; if still running, poll \
                       job_status with the returned job_id. Needs a vocal lane \
                       (section_set_lane_generator kind \"vocal\") with notes AND lyrics — \
                       pre-flight it with song_vocal and only render lanes whose \
                       counts_mismatch is false, since SVS is the most expensive operation on \
                       this surface.",
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
