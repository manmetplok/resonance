//! `bus_*` — group busses: the stage between tracks and master.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::bus;

#[tool_router(router = router_bus, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Create a group bus and return its bus_id IN THE REPLY, so you can route \
                       tracks into it on the very next call. \
                       \
                       A bus sums a group of tracks to one point before master. That buys two \
                       things faders cannot: you can process the group AS ONE — a single \
                       compressor across the whole kit glues it together in a way that \
                       per-track compressors never do — and you can move the group's level \
                       without disturbing the balance inside it. A drum bus is the usual first \
                       one. \
                       \
                       Busses show up in song_summary and song_tracks as entries with \
                       kind: \"bus\", from a distinct id range, so there is no separate list \
                       call. Route tracks in with track_set_output.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<bus::CreateResult>()
    )]
    async fn bus_create(
        &self,
        Parameters(params): Parameters<bus::CreateParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(bus::CREATE, &params).await
    }

    #[tool(
        description = "Delete a bus. Its member tracks are NOT deleted and NOT silenced — they \
                       fall back to routing straight to master — but the bus's own level and \
                       effect chain are gone, so the group will sound different. Refused with a \
                       list of the affected tracks until you pass confirm: true.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn bus_delete(
        &self,
        Parameters(params): Parameters<bus::DeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(bus::DELETE, &params).await
    }

    #[tool(
        description = "Set a bus's fader, in decibels (0 = unity, range -60..=+6). This is the \
                       lever for moving a whole group against the rest of the mix — pull the \
                       drum bus down 2 dB and every drum track keeps its internal balance \
                       exactly. song_summary reports the current value as the bus entry's \
                       volume_db. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn bus_set_volume(
        &self,
        Parameters(params): Parameters<bus::SetVolumeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(bus::SET_VOLUME, &params).await
    }
}
