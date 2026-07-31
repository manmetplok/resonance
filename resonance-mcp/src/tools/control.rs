//! `control_hello` — the connection handshake, exposed as a read-only
//! self-check tool.
//!
//! The client already handshakes on connect (see [`crate::client`]); this
//! tool re-asks so the *model* can see the answer. Its value is
//! diagnostic: `capabilities` is the method list the running server
//! declares, which is the only way to tell a tool that is *mis-called*
//! from a tool whose method the running build simply does not have —
//! the failure mode when the app binary is older than this MCP server.
//! It is a lower bound, not a guarantee: a declared method can still
//! answer `unsupported` (`render.stems` does, always).

use crate::server::ResonanceMcp;
use resonance_control::methods::control::{self, HelloParams, HelloResult};
use resonance_control::PROTOCOL_VERSION;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};

#[tool_router(router = router_control, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Self-check the connection: returns the running app's version, the \
                       control-protocol version, and capabilities — the list of control methods \
                       THIS running server declares, as dotted names \
                       (\"track.set_plugin_param\"). Read-only, cheap, safe to call any time. \
                       \
                       Every MCP tool here maps to one control method by turning the first \
                       underscore into a dot: track_plugin_params -> track.plugin_params, \
                       render_mixdown -> render.mixdown. So if a tool answers \"unsupported\", \
                       or behaves like an older build, check whether its method is in \
                       capabilities: if it is missing, no amount of re-phrasing the call will \
                       help — the app binary is older than this MCP server and both need to be \
                       rebuilt from the same checkout and the app restarted. \
                       \
                       capabilities is necessary but not sufficient: it lists what the PROTOCOL \
                       declares this server speaks, so a listed method can still answer \
                       unsupported — render.mixdown does that for a partial range, and \
                       render.stems is listed but not implemented at all. Treat a missing method \
                       as proof it cannot work, and a present one as no promise that it will.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<HelloResult>()
    )]
    async fn control_hello(&self) -> Result<CallToolResult, McpError> {
        let params = HelloParams {
            protocol_version: PROTOCOL_VERSION,
        };
        self.invoke_structured(control::HELLO, &params).await
    }
}
