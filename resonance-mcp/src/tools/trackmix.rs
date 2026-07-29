//! `track_*` / `mixer_*` — track lifecycle, per-track plugins, and mix
//! parameters. All mutations are normal undoable edits.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::{mixer, track};

#[tool_router(router = router_trackmix, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Add a track. kind: instrument | drums | vocal | audio; optional name. \
                       Returns the new track_id — use it in every later track/mixer/clip call. \
                       Give instrument tracks a sound with track_add_instrument.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<track::AddResult>()
    )]
    async fn track_add(
        &self,
        Parameters(params): Parameters<track::AddParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::ADD, &params).await
    }

    #[tool(
        description = "Rename a track (track_id from song_summary/song_tracks).",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn track_rename(
        &self,
        Parameters(params): Parameters<track::RenameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::RENAME, &params).await
    }

    #[tool(
        description = "Delete a track and everything on it. Destructive: refused with a \
                       summary of what would be lost until you pass confirm: true.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn track_delete(
        &self,
        Parameters(params): Parameters<track::DeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::DELETE, &params).await
    }

    #[tool(
        description = "Set a track's instrument to a built-in plugin by stable id (e.g. \
                       \"resonance-wavetable\") — list valid ids with track_plugins.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn track_add_instrument(
        &self,
        Parameters(params): Parameters<track::AddPluginParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::ADD_INSTRUMENT, &params).await
    }

    #[tool(
        description = "Append a built-in effect (e.g. \"resonance-reverb\") to a track's \
                       insert chain — list valid ids with track_plugins.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn track_add_effect(
        &self,
        Parameters(params): Parameters<track::AddPluginParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::ADD_EFFECT, &params).await
    }

    #[tool(
        description = "The built-in plugin catalog: stable plugin ids with name and kind \
                       (instrument | effect), for track_add_instrument / track_add_effect. \
                       Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<track::PluginCatalog>()
    )]
    async fn track_plugins(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::PLUGINS, &()).await
    }

    #[tool(
        description = "Set a track's fader gain. volume is linear: 1.0 = unity, 0.0 = silence.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn mixer_set_volume(
        &self,
        Parameters(params): Parameters<mixer::SetVolumeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(mixer::SET_VOLUME, &params).await
    }

    #[tool(
        description = "Set a track's stereo pan: -1.0 hard left .. 1.0 hard right, 0 center.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn mixer_set_pan(
        &self,
        Parameters(params): Parameters<mixer::SetPanParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(mixer::SET_PAN, &params).await
    }

    #[tool(
        description = "Mute or unmute a track.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn mixer_set_mute(
        &self,
        Parameters(params): Parameters<mixer::SetMuteParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(mixer::SET_MUTE, &params).await
    }

    #[tool(
        description = "Solo or unsolo a track (soloing silences all non-soloed tracks).",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn mixer_set_solo(
        &self,
        Parameters(params): Parameters<mixer::SetSoloParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(mixer::SET_SOLO, &params).await
    }
}
