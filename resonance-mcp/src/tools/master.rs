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

    #[tool(
        description = "Append an effect to the MASTER insert chain, where it processes the \
                       whole summed mix (after every track and bus, before the master fader). \
                       plugin_id is a CLAP id of the form \"com.resonance.<name>\"; \
                       \"com.resonance.mastering\" is the one to reach for here. Instrument \
                       plugins are refused — the master has no notes to play. Each call \
                       APPENDS, so calling it twice gives the master two instances (address \
                       them by occurrence in master_remove_effect). Undoable. \
                       \
                       WHY THIS MATTERS: a finished mix typically sits around -23..-18 LUFS \
                       with headroom deliberately left for mastering, and it CANNOT be made \
                       louder by raising faders — the loudest transient hits full scale first. \
                       A limiter on the master is what buys level beyond that transient. Do \
                       NOT chase loudness by pulling the drums down: that trades a level \
                       problem for a balance problem, and the balance problem is the one \
                       listeners hear. \
                       \
                       TARGETS: keep true peak at or below -1 dBTP (Spotify's recommendation), \
                       or -2 dBTP to stay safe through lossy codecs, which push peaks up. \
                       Streaming platforms normalise, so mastering louder than about -14 LUFS \
                       integrated buys nothing — it is turned back down on playback and you \
                       keep only the squashed dynamics.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn master_add_effect(
        &self,
        Parameters(params): Parameters<master::AddEffectParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(master::ADD_EFFECT, &params).await
    }

    #[tool(
        description = "Remove one effect from the master insert chain. Address it EITHER by \
                       slot (the 0-based position master_summary reports) OR by plugin_id plus \
                       occurrence when the same effect is on the master more than once — both \
                       forms together, or neither, is rejected rather than guessed at, because \
                       a guess here strips a processor off the entire mix. Remaining plugins \
                       renumber, so re-read master_summary before a second removal. Undoable.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn master_remove_effect(
        &self,
        Parameters(params): Parameters<master::RemoveEffectParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(master::REMOVE_EFFECT, &params).await
    }

    #[tool(
        description = "Bypass or re-engage the ENTIRE master effect chain in one call. This \
                       SETS the state rather than toggling it, so it is safe to repeat and \
                       cannot end up inverted; setting the state it is already in is a no-op. \
                       Bypassing is the A/B test for whether the master processing is helping: \
                       bounce with bypassed: true and with false and compare the measurements, \
                       rather than judging the chain by its settings. master_summary reports \
                       the current value as fx_bypassed. Master volume is NOT affected — only \
                       the plugins.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn master_set_fx_bypass(
        &self,
        Parameters(params): Parameters<master::SetFxBypassParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(master::SET_FX_BYPASS, &params).await
    }
}
