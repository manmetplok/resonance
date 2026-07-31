//! `transport_*` — playback control and global musical parameters.
//! Every tool returns the transport state after the call (state,
//! playhead, looping, revision).

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::transport;

#[tool_router(router = router_transport, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Start playback from the current playhead. The user hears the song in \
                       the running app. Use transport_seek first to choose where to play from.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<transport::TransportResult>()
    )]
    async fn transport_play(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(transport::PLAY, &()).await
    }

    #[tool(
        description = "Stop playback and return the playhead to the stop position.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<transport::TransportResult>()
    )]
    async fn transport_stop(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(transport::STOP, &()).await
    }

    #[tool(
        description = "Pause playback at the current position (resume with transport_play).",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<transport::TransportResult>()
    )]
    async fn transport_pause(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(transport::PAUSE, &()).await
    }

    #[tool(
        description = "Move the playhead to a musical position (bar 1-based, optional beat \
                       1-based within the bar) or an absolute sample position. Give either \
                       bar[+beat] or sample, not both.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<transport::TransportResult>()
    )]
    async fn transport_seek(
        &self,
        Parameters(params): Parameters<transport::SeekParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(transport::SEEK, &params).await
    }

    #[tool(
        description = "Set the loop region (start/end as bar[+beat] or sample positions); \
                       optional enabled also switches looping on/off in the same call.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<transport::TransportResult>()
    )]
    async fn transport_loop_set(
        &self,
        Parameters(params): Parameters<transport::LoopSetParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(transport::LOOP_SET, &params).await
    }

    #[tool(
        description = "Toggle looping on/off without changing the loop region.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<transport::TransportResult>()
    )]
    async fn transport_loop_toggle(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(transport::LOOP_TOGGLE, &()).await
    }

    #[tool(
        description = "Set the song tempo in BPM (undoable edit).",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<transport::TransportResult>()
    )]
    async fn transport_set_tempo(
        &self,
        Parameters(params): Parameters<transport::SetTempoParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(transport::SET_TEMPO, &params).await
    }

    #[tool(
        description = "Set the global time signature, e.g. numerator 3, denominator 4 for 3/4 \
                       (undoable edit).",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<transport::TransportResult>()
    )]
    async fn transport_set_time_signature(
        &self,
        Parameters(params): Parameters<transport::SetTimeSignatureParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(transport::SET_TIME_SIGNATURE, &params)
            .await
    }

    #[tool(
        description = "Set the global key: tonic is a pitch name (\"A\", \"F#\", \"Bb\"), scale \
                       is one of chromatic, major, minor, dorian, phrygian, lydian, mixolydian, \
                       locrian, \"harmonic minor\", \"melodic minor\" — an unknown value is \
                       rejected with the full list. If the song keeps key per-section instead, \
                       this reports unsupported — use section_set_scale.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<transport::TransportResult>()
    )]
    async fn transport_set_key(
        &self,
        Parameters(params): Parameters<transport::SetKeyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(transport::SET_KEY, &params).await
    }
}
