//! `song_*` — read-only introspection views. Never mutate; results are
//! structured content typed by the `resonance-control` view structs.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::song;

#[tool_router(router = router_song, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Whole-song overview: tempo (BPM), time signature, key (when global), \
                       length, transport state and playhead (bar.beat), the ordered section \
                       arrangement, and one summary line per track (id, name, kind, instrument, \
                       mute/solo/volume/pan, clip count). Read-only. Call this first — every \
                       other tool's ids come from here or the other song_* views.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<song::SongSummary>()
    )]
    async fn song_summary(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(song::SUMMARY, &()).await
    }

    #[tool(
        description = "Section definitions (id, name, length in bars, key/scale, chord grid \
                       with per-chord id/start_beat/duration/symbol) plus the arrangement \
                       placements (placement id, definition id, 1-based start bar). Read-only. \
                       Use before any section_*, harmony_* or generate_* call.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<song::SectionsView>()
    )]
    async fn song_sections(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(song::SECTIONS, &()).await
    }

    #[tool(
        description = "Per-track detail: summary fields plus the effect chain, frozen flag \
                       (frozen tracks reject note/lyric/instrument edits), and every clip \
                       (id, start position, length, midi flag). Omit track_id for all tracks. \
                       Read-only; clip_ids for song_notes and notes_* come from here.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<song::TracksView>()
    )]
    async fn song_tracks(
        &self,
        Parameters(params): Parameters<song::TracksParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(song::TRACKS, &params).await
    }

    #[tool(
        description = "The notes of one MIDI clip: index (how notes_edit/notes_delete address \
                       them), pitch as both MIDI number and name (60 = C4), start/duration in \
                       ticks and beats (clip-relative, 0-based), velocity. Optional range \
                       limits to notes overlapping a beat window — use it on big clips to keep \
                       output small. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<song::NotesView>()
    )]
    async fn song_notes(
        &self,
        Parameters(params): Parameters<song::NotesParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(song::NOTES, &params).await
    }

    #[tool(
        description = "A vocal track's lyric lines with per-syllable phonemes, the project's \
                       pronunciation overrides, and the SVS render state \
                       (not_rendered/rendering/rendered/stale/error). Omit track_id for the \
                       first vocal track. `lanes` lists the track's vocal lanes — one per \
                       (section, track) — with the section's definition_id and name, its \
                       start_bar, and the render pre-flight. SVS sings ONE SYLLABLE PER NOTE, \
                       and syllable_count is the app's own count of the syllables it will \
                       consume — trust it over counting words or syllables in the text yourself. \
                       counts_mismatch (syllable_count > 0 and != note_count) is the first check \
                       before vocal_render; note_count == 0 means the lane has no notes yet (run \
                       vocal_generate). \
                       \
                       This is the closest thing to HEARING the vocal without rendering it: the \
                       lane also reports its voicebank and comfortable_range, and per note the \
                       syllable, its phonemes, phoneme_count, pitch, duration_ms vs the \
                       min_duration_ms those phonemes need, and the too_short / out_of_range \
                       flags — summarised as short_note_count and out_of_range_note_count. \
                       Nonzero counts mean the line will be sung but not understood; the fix is \
                       more syllable breaks in the lyric or longer/retuned notes, not a \
                       synthesis setting. Check all of this BEFORE vocal_render, which is the \
                       most expensive operation on this surface. \
                       Pass a lane's definition_id as section_id to vocal_set_lyrics / \
                       vocal_set_line to write that specific lane. The top-level lines/ \
                       render_state describe the FIRST lane. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<song::VocalView>()
    )]
    async fn song_vocal(
        &self,
        Parameters(params): Parameters<song::VocalParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(song::VOCAL, &params).await
    }
}
