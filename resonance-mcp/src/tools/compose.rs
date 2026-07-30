//! `generate_*` / `notes_*` — the app's generators plus direct
//! piano-roll-level note editing.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::{generate, notes};

#[tool_router(router = router_compose, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Generate a pad, bass or lead part from a section's chord grid into a \
                       track (role: pad | bass | lead). The section needs chords first \
                       (harmony_apply_progression). Optional seed makes output reproducible. \
                       \
                       options is a role-specific object; every field is optional and any \
                       subset works. Without it you get the plain default (bass = a root on \
                       every beat), so set style to get a real part. \
                       bass: style (RootHold | RootPulse | RootFifth | Octave | Walking | \
                       Motif, default RootPulse), base_note (MIDI floor, default 28 = E1), \
                       velocity (0..1, default 0.85), and for style Motif: motif_mode \
                       (SameIntervals | Augmented | RhythmOnly | FirstNoteOnly), motif_phrase \
                       (Simple | MirrorMelody | Restricted). \
                       lead: style (ArpUp | ArpDown | ArpUpDown | Motif, default ArpUp), \
                       register ([low, high] MIDI, default [67, 88]), note_value_ticks (480 = \
                       quarters, 240 = 8ths default, 120 = 16ths), rest_density (0..1, default \
                       0), velocity (default 0.8), fill_vocal_gaps (sound only where the \
                       section's vocal lane is silent), and for style Motif: complexity (0..1, \
                       default 0.5), articulation (0 legato .. 1 staccato, default 0.3), \
                       contour (Auto | Arch | Descending | Ascending | Wave), phrase_len (2 | \
                       4 | 8, default 4), motif_len (0 = auto), leap_chance (default 0.21), \
                       embellishment (Auto | Folk | PopBallad | Jazz). \
                       pad: register ([low, high] MIDI, default [52, 76]), velocity (default \
                       0.7). \
                       \
                       Returns clip_ids, one per placement of the section in arrangement \
                       order, and clip_id as the first of them — no follow-up song_tracks \
                       call needed.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<generate::GenerateResult>()
    )]
    async fn generate_part(
        &self,
        Parameters(params): Parameters<generate::PartParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(generate::PART, &params).await
    }

    #[tool(
        description = "Generate a drum pattern for one section onto a drums track. Only that \
                       section's drums change — other sections keep their material even when \
                       they started from the same pattern. Optional pattern names a style; \
                       omitted uses the section's arrangement default. Call it once per \
                       section to give each its own groove. Returns clip_ids, one per \
                       placement of the section in arrangement order, and clip_id as the \
                       first of them.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<generate::GenerateResult>()
    )]
    async fn generate_drums(
        &self,
        Parameters(params): Parameters<generate::DrumsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(generate::DRUMS, &params).await
    }

    #[tool(
        description = "Insert one note into a MIDI clip: pitch as MIDI number (60 = C4), \
                       0-based clip-relative start_beat, duration_beats, velocity 1-127 \
                       (default 100). Returns the note's index. clip_id from song_tracks or \
                       notes_create_clip.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<notes::InsertResult>()
    )]
    async fn notes_insert(
        &self,
        Parameters(params): Parameters<notes::InsertParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(notes::INSERT, &params).await
    }

    #[tool(
        description = "Edit one note in place, addressed by clip_id + index from song_notes; \
                       omitted fields stay unchanged. Edits can reorder the note list — \
                       re-read song_notes before further index-based edits.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn notes_edit(
        &self,
        Parameters(params): Parameters<notes::EditParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(notes::EDIT, &params).await
    }

    #[tool(
        description = "Delete one note, addressed by clip_id + index from song_notes. Later \
                       indices shift down — re-read song_notes before deleting more.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn notes_delete(
        &self,
        Parameters(params): Parameters<notes::DeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(notes::DELETE, &params).await
    }

    #[tool(
        description = "Create an empty MIDI clip on a track, positioned either inside a \
                       section placement (placement_id from song_sections — length defaults to \
                       the section) or at an explicit 1-based start_bar. Returns clip_id for \
                       notes_insert.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<notes::CreateClipResult>()
    )]
    async fn notes_create_clip(
        &self,
        Parameters(params): Parameters<notes::CreateClipParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(notes::CREATE_CLIP, &params).await
    }

    #[tool(
        description = "Move an existing MIDI clip to a new timeline position: give either a \
                       1-based start_bar or a placement_id to anchor it to that section \
                       placement's start. The target snaps to the bar grid, so this also \
                       re-grids a clip whose start drifted. Track, length and notes are \
                       unchanged.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn notes_move_clip(
        &self,
        Parameters(params): Parameters<notes::MoveClipParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(notes::MOVE_CLIP, &params).await
    }

    #[tool(
        description = "Insert a BATCH of notes into a MIDI clip as one undoable edit. Prefer \
                       this over repeated notes_insert: each call is its own undo entry, so a \
                       400-note part written note-by-note leaves 400 of them. Each note is \
                       {pitch, start_beat, duration_beats, velocity?}. Returns the index each \
                       submitted note landed at, in the order given.",
        annotations(destructive_hint = false, idempotent_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<notes::InsertManyResult>()
    )]
    async fn notes_insert_many(
        &self,
        Parameters(params): Parameters<notes::InsertManyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(notes::INSERT_MANY, &params).await
    }

    #[tool(
        description = "Replace a MIDI clip's ENTIRE note list with the given notes, as one \
                       undoable edit. Destructive: every existing note in the clip is dropped. \
                       Use it to rewrite a part wholesale instead of deleting note-by-note. \
                       Returns the index each submitted note landed at, in the order given.",
        annotations(destructive_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<notes::InsertManyResult>()
    )]
    async fn notes_replace_all(
        &self,
        Parameters(params): Parameters<notes::ReplaceAllParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(notes::REPLACE_ALL, &params).await
    }
}
