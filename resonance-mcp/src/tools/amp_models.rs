//! `amp_models_*` — the user's installed NAM amp models, the library
//! Resonance Amp loads from (nam-model-library.md §9.3).
//!
//! One tool per `amp_models.*` control method
//! (`resonance_control::methods::amp_models::METHODS`).

use crate::server::ResonanceMcp;
use resonance_control::methods::amp_models;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};

#[tool_router(router = router_amp_models, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "List the NAM amp models installed for Resonance Amp (com.resonance.amp): \
                       the user's per-machine model library, not part of the project, so no \
                       project needs to be open. Each model reports slot, id, name, author, \
                       gear, gear_type, tone_type (capture type: clean, crunch, hi_gain, …), \
                       architecture, sample_rate, size_bytes, source (tone3000 / imported / \
                       external), favorite, tags, last_used and status (\"unreadable\" models \
                       carry the parse error). Favourites come first, then slot order. \
                       \
                       Filters AND: query (the amp Library panel's search — tokens matched \
                       against name, author, gear, file name and tags; is:fav, is:recent, \
                       tag:<t>, by:<author>, is:tone3000, gear_type:<g>, tone_type:<t> scope a \
                       token), favorites_only, gear_type, tone_type. \
                       \
                       To load one, set the amp's \"Model Select\" parameter with \
                       track_set_plugin_param (or bus_/master_): value = the entry's slot, or \
                       its name, which the plugin resolves. The slot is stable — adding or \
                       deleting models never moves it. A model captured at another \
                       sample_rate than the session runs with a shifted tone. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<amp_models::AmpModelList>()
    )]
    async fn amp_models_list(
        &self,
        Parameters(params): Parameters<amp_models::ListParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(amp_models::LIST, &params).await
    }

    #[tool(
        description = "Star or tag one installed NAM amp model: favorite (true/false) and/or \
                       tags (REPLACES its personal tags; [] clears them; normalised to \
                       lowercase a-z0-9-). id is the model's id from amp_models_list (a \
                       unique prefix of 8+ characters works). Returns the updated model. \
                       \
                       This is the user's own per-machine state, shared with the amp's \
                       Library panel — NOT a project edit: it records no undo entry \
                       (edit_undo does not take it back) and does not bump the project \
                       revision. Favourites sort first in amp_models_list and in the amp's \
                       ◀/▶ stepping.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        ),
        output_schema = schema_for_output::<amp_models::AmpModelEntry>()
    )]
    async fn amp_models_set_marks(
        &self,
        Parameters(params): Parameters<amp_models::SetMarksParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(amp_models::SET_MARKS, &params).await
    }
}
