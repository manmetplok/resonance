//! `drum_kits_*` — the user's installed drum kits, the library Resonance
//! Drums loads from (drums-plugin-rework.md §8).
//!
//! One tool per `drum_kits.*` control method
//! (`resonance_control::methods::drum_kits::METHODS`).

use crate::server::ResonanceMcp;
use resonance_control::methods::drum_kits;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};

#[tool_router(router = router_drum_kits, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "List the drum kits installed for Resonance Drums (com.resonance.drums): \
                       the user's per-machine kit library, not part of the project, so no \
                       project needs to be open. Each kit reports slot, id, name, description, \
                       pieces (count) and piece_names, mic_setups (count), layers (most \
                       velocity layers), rr (most round-robin takes), size_bytes (once \
                       measured), source (plok / imported / local), favorite, tags, last_used, \
                       status (ok / manifest_error / missing_files / duplicate, with error) and \
                       loaded_in (the open project's tracks whose drums have it selected). \
                       Favourites come first, then slot order; limit caps the list and matched \
                       counts every match. \
                       \
                       Filters AND: query (the drums Library search — tokens matched against \
                       the kit name, description, piece names, mic brands/models/positions and \
                       tags; is:fav, is:recent, tag:<t>, is:plok, mics:<1|2–4|5+>, \
                       articulations:<yes|no> scope a token), favorites_only, source. \
                       \
                       To load one, set the drums' kit_select parameter with \
                       track_set_plugin_param to the kit's name (or its slot), then poll \
                       track_plugin_params until kit_load_progress reads 1.0 — it completes \
                       with the transport stopped too; its text says \"failed\" if the load \
                       did. kit_select -1 is the small built-in kit. Installing, importing \
                       and deleting kits happen in the drums editor only. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<drum_kits::DrumKitList>()
    )]
    async fn drum_kits_list(
        &self,
        Parameters(params): Parameters<drum_kits::ListParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(drum_kits::LIST, &params).await
    }

    #[tool(
        description = "Star or tag one installed drum kit: favorite (true/false) and/or tags \
                       (REPLACES its personal tags; [] clears them; normalised to lowercase \
                       a-z0-9-). id is the kit's id from drum_kits_list (a unique prefix of 8+ \
                       characters works). Returns the updated kit. \
                       \
                       This is the user's own per-machine state, shared with the drums' \
                       Library overlay — NOT a project edit: it records no undo entry \
                       (edit_undo does not take it back) and does not bump the project \
                       revision. Favourites sort first in drum_kits_list and in the drums' \
                       ◀/▶ stepping.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        ),
        output_schema = schema_for_output::<drum_kits::DrumKitEntry>()
    )]
    async fn drum_kits_set_marks(
        &self,
        Parameters(params): Parameters<drum_kits::SetMarksParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(drum_kits::SET_MARKS, &params).await
    }
}
