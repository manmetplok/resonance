//! `automation_*` — parameter automation lanes: a fader, pan, mute or
//! plugin parameter that changes over the song.
//!
//! One tool per `automation.*` control method the app implements
//! (`resonance_control::methods::automation::METHODS`); the remaining
//! methods of automation-control-api.md §4.4 join as their handlers land.

use crate::server::ResonanceMcp;
use resonance_control::methods::automation;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};

#[tool_router(router = router_automation, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Read automation lanes — a fader, pan, mute or plugin parameter that \
                       changes over the song. One lane per target. With no params, every lane \
                       in the project, orphans included; each filter narrows: track_id / \
                       bus_id / master: true (the lanes that owner carries, including lanes \
                       on its plugins), control (\"volume\" | \"pan\" | \"mute\" | \"device\"), \
                       param (name, numeric id or string key), plugin_id / occurrence. range \
                       {start, end} (half-open) windows the points; each point keeps its \
                       whole-lane index. \
                       \
                       Each lane reports its target (pass it straight back to any \
                       automation_* tool), enabled (the Read flag), status (\"ok\"; \
                       \"orphaned\" = its plugin or track is gone, inert; \"plugin_missing\"; \
                       \"plugin_initializing\" = retry shortly), unit, min, max and points. A \
                       point reads {index, position: {bar, beat, sample}, value, normalized, \
                       text, curve}. Values are REAL units: volume dB (-60..=6; the floor \
                       reads \"-inf\", silence), pan -1..=1, mute true/false, a plugin \
                       parameter in its own min..=max (text carries a choice label). Bars and \
                       beats are 1-based and meter-aware — beat counts the time signature's \
                       beat unit, so a reported position can be sent straight back. \
                       \
                       TEMPO / METER: a point is anchored at a SAMPLE. transport_set_tempo and \
                       arrangement_insert_bars / arrangement_remove_bars re-anchor lanes, so a \
                       point at bar 17 stays at bar 17. The global_* tempo and signature \
                       events do NOT: lanes keep their sample position like clips and markers, \
                       so their bar position moves — re-read after such an edit. \
                       \
                       You cannot hear automation: verify with automation_lanes; measure with \
                       meter_measure over the range. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<automation::LanesResult>()
    )]
    async fn automation_lanes(
        &self,
        Parameters(params): Parameters<automation::LanesParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(automation::LANES, &params).await
    }

    #[tool(
        description = "Create an automation lane, or REPLACE ALL of its points — whatever the \
                       lane held before is gone (undoable, one undo entry). Target: exactly one \
                       owner (track_id, bus_id or master: true) and exactly one of control \
                       (\"volume\" | \"pan\" | \"mute\"; master: volume only) or param (a \
                       plugin parameter by name, numeric id or string key — plugin_id omitted \
                       = the track's instrument, or a bus's / the master's first plugin; \
                       occurrence picks among same-id plugins). \
                       \
                       points: [{position: {bar, beat} | {sample}, value, curve?}], at least \
                       one (an empty list is refused — use automation_remove_lane to delete a \
                       lane), at most 2048, any order; two points on the same position are \
                       refused. Values are REAL units unless normalized: true (then 0..=1 for \
                       the whole call): volume dB -60..=6 or \"-inf\" (silence), pan -1..=1, \
                       mute true/false, a plugin parameter in its own min..=max or a choice \
                       label. Out-of-range values are refused with the range. curve \
                       \"linear\" (default) or \"stepped\" (hold until the next point; the \
                       default for mute and stepped parameters). Plugin lanes are LINEAR IN \
                       THE PARAMETER'S PLAIN UNITS, so two points 300 Hz -> 4 kHz sweep \
                       linearly in Hz. enabled sets the Read flag (default: keep, new lanes \
                       on). Bars and beats are 1-based and meter-aware (beat counts the time \
                       signature's beat unit). \
                       \
                       TEMPO / METER: points are anchored at a sample. transport_set_tempo and \
                       arrangement_insert/remove_bars keep a point at its bar; global_* tempo \
                       and signature events keep its sample, so its bar moves. \
                       \
                       A plugin lane on a FROZEN track is refused (unfreeze first); volume / \
                       pan / mute lanes stay writable. Returns {revision, lane} — the lane \
                       exactly as it reads back. You cannot hear it: verify with \
                       automation_lanes; measure with meter_measure over the range.",
        annotations(destructive_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<automation::LaneEditResult>()
    )]
    async fn automation_set_lane(
        &self,
        Parameters(params): Parameters<automation::SetLaneParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(automation::SET_LANE, &params).await
    }
}
