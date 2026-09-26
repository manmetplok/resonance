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
        description = "Generate a pad, bass or lead part from a section's chord grid into a synth \
                       instrument track (role: pad | bass | lead). A drum or vocal track is \
                       rejected — use generate_drums / vocal_generate. The section needs chords \
                       first (harmony_apply_progression) or the call is refused. seed makes the \
                       output reproducible; omitted it is derived from the section id, so \
                       repeating the call is stable rather than random. \
                       \
                       Omit chord_count, beats_per_chord and sevenths: the generator always \
                       plays the section's chord grid exactly as it stands, so setting any of \
                       them is REJECTED. Shape the harmony with harmony_apply_progression \
                       (which takes beats_per_chord and sevenths) before calling this. \
                       \
                       options is a role-specific object; every field is optional and any \
                       subset works, so `{\"style\": \"Walking\"}` is a valid whole object. \
                       Enum values are spelled exactly as below (PascalCase, case-sensitive). \
                       WITHOUT options you get the plain default, and for bass that default is \
                       RootPulse — a literal root note on every beat, which is a placeholder, \
                       not a bass line. Set style. \
                       \
                       bass: style (RootHold = one held note per chord | RootPulse = root on \
                       every beat, DEFAULT | RootFifth | Octave | Walking = scale-stepping line \
                       approaching the next chord root, needs the section to have a scale | \
                       Motif = develops the section's shared motif), base_note (MIDI floor, \
                       default 28 = E1), velocity (0..1, default 0.85), and for style Motif only: \
                       motif_mode (SameIntervals | Augmented | RhythmOnly | FirstNoteOnly), \
                       motif_phrase (Simple | MirrorMelody | Restricted). Walking and Motif are \
                       the two that produce an actual part. \
                       \
                       lead: style (ArpUp DEFAULT | ArpDown | ArpUpDown | Motif = real melodic \
                       development with phrasing and contour), register ([low, high] MIDI, \
                       default [67, 88]), note_value_ticks (480 = quarters, 240 = 8ths DEFAULT, \
                       120 = 16ths; arp styles only), rest_density (0..1, default 0; arp styles \
                       only), velocity (default 0.8), fill_vocal_gaps (bool, sound only where \
                       the section's vocal lane is silent — call-and-response), and for style \
                       Motif only: complexity (0..1, default 0.5), articulation (0 legato .. 1 \
                       staccato, default 0.3), contour (Auto | Arch | Descending | Ascending | \
                       Wave), phrase_len (2 | 4 | 8, default 4), motif_len (0 = auto), \
                       leap_chance (default 0.21), embellishment (Auto | Folk | PopBallad | \
                       Jazz). \
                       \
                       pad: register ([low, high] MIDI, default [52, 76]), velocity (default \
                       0.7). Pad has no style — it always voices the chords SATB-style. \
                       \
                       An unparseable options object is rejected with the serde error naming the \
                       offending field. Returns clip_ids, one per placement of the section in \
                       arrangement order, and clip_id as the first of them — no follow-up \
                       song_tracks call needed. Verify with song_notes on the returned clip_id.",
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
        description = "Generate a drum pattern for one section onto a drums track (track_id must \
                       be a drum track — any other kind is rejected). Only that section's drums \
                       change: other sections keep their material even when they started from \
                       the same pattern, so call it once per section to give each its own \
                       groove. \
                       \
                       pattern names the groove. It resolves against the project's own pattern \
                       bank first, then a built-in library: silence (no drums — pins the \
                       section drumless, otherwise an un-generated section picks up the \
                       project default), halftime (backbeat on 3), \
                       four-on-floor, industrial (rigid gated 16ths), breakbeat (syncopated, \
                       ghosted), blast, sparse (kick on 1, snare on 3), toms (tom-led, no \
                       hats), build (one-bar ramp into 16ths), fill (tom fill onto a crash). \
                       Built-ins install exactly as authored, so they are reproducible and \
                       seed does not change them; omitting pattern rolls the section's \
                       existing bank pattern instead, where seed does apply. \
                       \
                       density (0.0..=1.0, default 1.0) sets how busy the result is: on a \
                       built-in it thins the groove toward the strong beats without losing any \
                       voice, so the same name at 0.3 / 0.6 / 1.0 across three sections reads \
                       as one idea getting busier. \
                       \
                       An unknown pattern name is rejected with both lists (project bank and \
                       built-ins) spelled out, so a wrong guess is self-correcting. \
                       \
                       Returns clip_ids, one per placement of the section in arrangement \
                       order, and clip_id as the first of them. Verify with song_notes on the \
                       returned clip_id.",
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
                       undoable edit. Destructive: every existing note in the clip is dropped, \
                       so it follows the confirm convention — on a non-empty clip it is \
                       refused with the existing note count until you re-send with \
                       confirm: true (an empty clip loses nothing and needs no confirm). \
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

    #[tool(
        description = "Import a Standard MIDI File \u{2014} the CHEAP path for bulk material. A \
                       part written through notes_insert_many travels as a JSON note array in \
                       the tool call; a few thousand notes that way dominate the cost of the \
                       whole session. An SMF carries the same part in a fraction of the payload \
                       and lands as ONE undoable edit. \
                       \
                       Give the file as `path` (an absolute path on the machine running the \
                       app) OR as `data_base64` \u{2014} one or the other, never both. Files up \
                       to 4 MB are accepted; anything larger is refused rather than truncated, \
                       so split it. Roughly, keep base64 payloads under a megabyte or so and \
                       prefer `path` when the file is already on disk. \
                       \
                       Target it at a `track_id` (with an optional 1-based `start_bar`, default \
                       bar 1), which creates a clip long enough to hold the part, or at an \
                       existing `clip_id`, whose notes are REPLACED. A multi-track SMF is \
                       refused unless you name `source_track`, and the error lists every track \
                       with its name and note count so you can pick \u{2014} it is never \
                       silently flattened into one clip. Note positions honour the project\'s \
                       tempo map, not a fixed BPM. The result reports clip_id, note_count and \
                       length_beats, so no follow-up song_notes is needed.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<notes::ImportMidiResult>()
    )]
    async fn notes_import_midi(
        &self,
        Parameters(params): Parameters<notes::ImportMidiParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(notes::IMPORT_MIDI, &params).await
    }
}
