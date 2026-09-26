//! `section_*` / `harmony_*` — song structure (section definitions and
//! placements) and the chord grid, including music-theory progression
//! application.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::{harmony, section};

#[tool_router(router = router_arrange, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Create a section definition (name, length_bars, optional key/scale) and, \
                       BY DEFAULT, also place it on the timeline right after the last existing \
                       placement. \
                       \
                       PASS place: false whenever you are building an arrangement in a \
                       deliberate order. The implicit placement is the single biggest trap on \
                       this surface: leave place at its default and the follow-up section_place \
                       is rejected as overlapping, because the section is already on the \
                       timeline at a bar you did not choose. The reliable pattern is \
                       section_create {place: false} for every section, then section_place each \
                       one at its 1-based start_bar. Leave place at true only when appending \
                       sections strictly front-to-back and \"after the last one\" is where you \
                       want this one. \
                       \
                       Returns section_id. Definitions — not placements — carry the chord grid \
                       every harmony_* and generate_* call reads. scale is {tonic, scale}, e.g. \
                       {\"tonic\": \"A\", \"scale\": \"minor\"}; scale is one of chromatic, \
                       major, minor, dorian, phrygian, lydian, mixolydian, locrian, \
                       \"harmonic minor\", \"melodic minor\".",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<section::CreateResult>()
    )]
    async fn section_create(
        &self,
        Parameters(params): Parameters<section::CreateParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(section::CREATE, &params).await
    }

    #[tool(
        description = "Rename a section definition (section_id from song_sections).",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn section_rename(
        &self,
        Parameters(params): Parameters<section::RenameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(section::RENAME, &params).await
    }

    #[tool(
        description = "Change a section definition's length in bars (affects every placement \
                       of it).",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn section_resize(
        &self,
        Parameters(params): Parameters<section::ResizeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(section::RESIZE, &params).await
    }

    #[tool(
        description = "Delete a section definition AND all its placements. Destructive: \
                       refused with a summary until you pass confirm: true. To remove one \
                       occurrence from the timeline use section_remove_placement instead.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn section_delete(
        &self,
        Parameters(params): Parameters<section::DeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(section::DELETE, &params).await
    }

    #[tool(
        description = "Place a section definition in the arrangement at a 1-based start bar. \
                       Returns placement_id. The same definition can be placed multiple times \
                       (verse 1, verse 2, ...).",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<section::PlaceResult>()
    )]
    async fn section_place(
        &self,
        Parameters(params): Parameters<section::PlaceParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(section::PLACE, &params).await
    }

    #[tool(
        description = "Remove one placement from the arrangement; the definition (and its \
                       chords) survives.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn section_remove_placement(
        &self,
        Parameters(params): Parameters<section::RemovePlacementParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(section::REMOVE_PLACEMENT, &params).await
    }

    #[tool(
        description = "Set a section definition's key/scale, e.g. tonic \"A\", scale \"minor\". \
                       Generators (generate_part, vocal_generate) and roman-numeral progression \
                       rendering read it. tonic is a pitch name (\"A\", \"F#\", \"Bb\"); scale is \
                       one of chromatic, major, minor, dorian, phrygian, lydian, mixolydian, \
                       locrian, \"harmonic minor\", \"melodic minor\" (matched \
                       case-insensitively, with - and _ treated as spaces). An unknown value is \
                       rejected with the full list.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn section_set_scale(
        &self,
        Parameters(params): Parameters<section::SetScaleParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(section::SET_SCALE, &params).await
    }

    #[tool(
        description = "Configure (or with kind \"manual\", clear) the generator on one \
                       (section, track) lane. kind: bass | melody | pad on a synth instrument \
                       track, vocal on a vocal track, manual to remove the lane's generator — \
                       the wrong track kind is rejected precisely. Installs the generator only: \
                       it derives NO notes. To get melodic MIDI now, call generate_part \
                       (bass/lead/pad) or vocal_generate instead; this tool is for the vocal \
                       lane and for pinning a lane's generator config. \
                       \
                       It is the ONLY way to create the vocal lane that every vocal_* tool \
                       needs. Lyrics live per (section, track) vocal lane and vocal_set_lyrics \
                       targets a track's FIRST vocal lane, so a song that sings in four sections \
                       needs four vocal tracks — one vocal lane each. \
                       \
                       options is the same per-kind object generate_part documents (bass -> \
                       BassParams, melody -> the same params generate_part's role \"lead\" takes, \
                       pad -> PadParams); a partial object is fine, omitted fields keep the \
                       generator defaults. EXCEPTION: for kind \"vocal\" the options object must \
                       be COMPLETE — it has no field defaults, so any partial object is rejected \
                       with a serde \"missing field\" error. Omit options entirely for vocal \
                       lanes and shape the result with vocal_generate + notes_* instead.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<section::SetLaneGeneratorResult>()
    )]
    async fn section_set_lane_generator(
        &self,
        Parameters(params): Parameters<section::SetLaneGeneratorParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(section::SET_LANE_GENERATOR, &params)
            .await
    }

    #[tool(
        description = "Add one chord to a section's grid: 0-based start_beat within the \
                       section, duration in beats, symbol like \"Am7\". Returns chord_id. For \
                       whole progressions prefer harmony_apply_progression.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<harmony::AddChordResult>()
    )]
    async fn harmony_add_chord(
        &self,
        Parameters(params): Parameters<harmony::AddChordParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(harmony::ADD_CHORD, &params).await
    }

    #[tool(
        description = "Edit a chord's symbol, start_beat and/or duration_beats; omitted fields \
                       stay unchanged (chord_id from song_sections).",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn harmony_edit_chord(
        &self,
        Parameters(params): Parameters<harmony::EditChordParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(harmony::EDIT_CHORD, &params).await
    }

    #[tool(
        description = "Remove one chord from a section's grid.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn harmony_delete_chord(
        &self,
        Parameters(params): Parameters<harmony::DeleteChordParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(harmony::DELETE_CHORD, &params).await
    }

    #[tool(
        description = "Write a whole chord progression onto a section's grid, REPLACING its \
                       existing chords. Give EXACTLY ONE chord source: \
                       symbols ([\"Am7\",\"Dm7\",\"G7\",\"Cmaj7\"]), or key + numerals \
                       ([\"i\",\"VI\",\"III\",\"VII\"] — case is ignored, quality comes from the \
                       key), or key + preset. Supplying two, or a numeral/preset without key, is \
                       rejected. \
                       \
                       preset is one of: pop, axis (both I V vi IV), 50s, doo-wop (both I vi IV \
                       V), pachelbel, andalusian, ii-V-I, 12-bar-blues (case-insensitive). \
                       key is {tonic, scale} with scale one of chromatic, major, minor, dorian, \
                       phrygian, lydian, mixolydian, locrian, \"harmonic minor\", \
                       \"melodic minor\". \
                       \
                       beats_per_chord must be a whole number of beats and defaults to one bar \
                       per chord; the progression must fit the section's length or it is \
                       rejected with the arithmetic. sevenths enriches numeral/preset voicings \
                       only. Returns the new chord ids in order.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<harmony::ApplyProgressionResult>()
    )]
    async fn harmony_apply_progression(
        &self,
        Parameters(params): Parameters<harmony::ApplyProgressionParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(harmony::APPLY_PROGRESSION, &params)
            .await
    }
}
