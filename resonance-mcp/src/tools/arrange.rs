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
        description = "Create a section definition (e.g. \"Verse\", 8 bars, optional \
                       key/scale). Returns section_id. Definitions carry the chord grid; put \
                       them on the timeline with section_place.",
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
                       Generators and progression rendering use this.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn section_set_scale(
        &self,
        Parameters(params): Parameters<section::SetScaleParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(section::SET_SCALE, &params).await
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
                       existing chords. Give either explicit symbols ([\"Am7\",\"Dm7\",\"G7\", \
                       \"Cmaj7\"]) or key + roman numerals ([\"i\",\"VI\",\"III\",\"VII\"]) or a \
                       named preset; beats_per_chord defaults to one bar per chord; sevenths \
                       enriches numeral/preset voicings. Returns the new chord ids in order.",
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
