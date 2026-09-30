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
        description = "Search plugin presets across every plugin (or one: plugin_id), before \
                       a plugin is even on a track — pick a plugin AND a preset in one read. \
                       Filters AND: query (tokens match name, author, description, category \
                       and tags; is:fav, is:recent, is:user, is:factory, tag:, genre:, cat:, \
                       for:, char:, by: scope a token to a value or its prefix, e.g. \
                       \"for:vocal char:warm\"), \
                       favorites_only, source, category, instrument, genres, character, tags; \
                       sort (bank / name / category / recent / modified); limit (default 100) \
                       and offset. Each hit carries plugin_id and the preset entry (id, \
                       metadata, favorite, tags). Add the plugin with track_add_effect / \
                       track_add_instrument / bus_add_effect / master_add_effect, passing the \
                       hit's id as the preset string (preset: \"<id>\"), or load it onto an \
                       existing one with track_load_plugin_preset (preset_id). Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<presets::SearchResult>()
    )]
    async fn presets_search(
        &self,
        Parameters(params): Parameters<presets::SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(presets::SEARCH, &params).await
    }

    #[tool(
        description = "Rename a USER preset (by plugin_id + preset_id). Its id does not \
                       change, so stars, tags and every project that loaded it follow. Names \
                       are unique per plugin among user presets (case-insensitively): a name \
                       another user preset has is refused. Factory presets cannot be renamed. \
                       Library state, not the project: no undo entry, no revision bump.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        ),
        output_schema = schema_for_output::<presets::EntryResult>()
    )]
    async fn presets_rename(
        &self,
        Parameters(params): Parameters<presets::RenameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(presets::RENAME, &params).await
    }

    #[tool(
        description = "Delete a USER preset: it moves to the preset trash, recoverable for 30 \
                       days, and its star survives a restore. Destructive, so without \
                       confirm: true the call is refused with what would be lost. Factory \
                       presets cannot be deleted. Library state, not the project: edit_undo \
                       does not take it back.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        ),
        output_schema = schema_for_output::<presets::DeleteResult>()
    )]
    async fn presets_delete(
        &self,
        Parameters(params): Parameters<presets::DeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(presets::DELETE, &params).await
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
