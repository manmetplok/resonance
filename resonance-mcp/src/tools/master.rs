//! `master_*` — the master bus: the final summing stage every track and
//! bus feeds into.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::master;

#[tool_router(router = router_master, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "The master bus: its fader (linear volume and volume_db), whether its FX \
                       chain is bypassed, and every plugin inserted on it with its slot, CLAP \
                       id and name. The master is the FINAL summing stage — every track and \
                       every bus lands here, after their own faders and effects — so anything \
                       inserted here acts on the whole mix at once. Read this before deciding \
                       what to put on the master: an empty plugins array means the mix is \
                       going out completely unprocessed. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<master::MasterSummary>()
    )]
    async fn master_summary(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(master::SUMMARY, &()).await
    }

    #[tool(
        description = "Set the master fader. Give EXACTLY ONE of volume (linear gain, 1.0 = \
                       unity) or volume_db (decibels, 0 = unity); both or neither is rejected. \
                       Accepted range is -60..=+6 dB (linear 0.0..=~1.995). \
                       \
                       This is a single scalar applied after everything else, so it does NOT \
                       change the balance between tracks — every relationship is preserved \
                       exactly. It also cannot make a mix meaningfully louder: raising it \
                       raises the loudest transient with everything else, so the peak hits \
                       full scale and clips long before the average level catches up. Getting \
                       a mix louder without clipping needs dynamics processing on the master \
                       chain (a limiter), not a bigger master fader. Undoable, and reflected \
                       in the GUI's master strip.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn master_set_volume(
        &self,
        Parameters(params): Parameters<master::SetMasterVolumeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(master::SET_VOLUME, &params).await
    }
}
