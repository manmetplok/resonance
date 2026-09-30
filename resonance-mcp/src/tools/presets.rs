//! `presets_*` — the user's plugin preset library across every plugin
//! (plugin-preset-library.md §12.2).
//!
//! One tool per `presets.*` control method
//! (`resonance_control::methods::presets::METHODS`).

use crate::server::ResonanceMcp;
use resonance_control::methods::presets;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};

#[tool_router(router = router_presets, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Star and/or personally tag one plugin preset — factory presets \
                       included (the mark never touches the preset itself). plugin_id is the \
                       plugin's CLAP id, preset_id the preset's stable id from \
                       track_plugin_presets (or bus_/master_). favorite: true/false; tags \
                       REPLACES the personal tags ([] clears; normalised to lowercase \
                       a-z0-9-). Returns the updated entry. \
                       \
                       Per-user library state shared with every plugin's own preset browser, \
                       NOT a project edit: no undo entry, no project revision bump. Filter on \
                       it with favorites_only / is:fav / tag:<t> in *_plugin_presets.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        ),
        output_schema = schema_for_output::<presets::EntryResult>()
    )]
    async fn presets_set_marks(
        &self,
        Parameters(params): Parameters<presets::SetMarksParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(presets::SET_MARKS, &params).await
    }

    #[tool(
        description = "Edit a USER preset's own metadata — what travels with the preset file: \
                       set replaces any of author, description, category, instrument, genres, \
                       character, tags (an omitted field is left alone); add_tags / \
                       remove_tags edit the content tags. The name is not changed here. \
                       Refused on a factory preset (read-only: use presets_set_marks for a \
                       star or personal tags). Use presets_vocabulary for the seeded values \
                       so tags stay consistent. Library state, not the project: no undo \
                       entry, no revision bump.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        ),
        output_schema = schema_for_output::<presets::EntryResult>()
    )]
    async fn presets_update_meta(
        &self,
        Parameters(params): Parameters<presets::UpdateMetaParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(presets::UPDATE_META, &params).await
    }

    #[tool(
        description = "The preset metadata vocabulary: the seeded categories (for instrument \
                       and for effect plugins), instrument (\"what it is for\": vocal, \
                       drum-bus, synth-bass, …), genres and character (timbre words: warm, \
                       dark, wide, …) values, each followed by values already in use, plus \
                       the tags in use. Tag and filter with these instead of inventing \
                       near-duplicates; a new value is still accepted. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<presets::Vocabulary>()
    )]
    async fn presets_vocabulary(
        &self,
        Parameters(params): Parameters<presets::VocabularyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(presets::VOCABULARY, &params).await
    }
}
