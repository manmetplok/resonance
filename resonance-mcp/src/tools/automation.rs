//! `automation_*` — parameter automation lanes: a fader, pan, mute or
//! plugin parameter that changes over the song.
//!
//! One tool per `automation.*` control method the app implements
//! (`resonance_control::methods::automation::METHODS`); `automation_shape`
//! (automation-control-api.md §4.6) joins as its handler lands.

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

    #[tool(
        description = "Insert automation points into a lane, creating it when it does not \
                       exist yet. A point on a frame the lane already holds REPLACES that \
                       point in place (counted in the reply's replaced), rather than adding a \
                       second point at the same position; two input points that resolve to the \
                       same position are refused, naming both. Target: exactly one owner \
                       (track_id, bus_id or master: true) and exactly one of control \
                       (\"volume\" | \"pan\" | \"mute\"; master: volume only) or param (a \
                       plugin parameter by name, numeric id or string key). \
                       \
                       points: [{position: {bar, beat} | {sample}, value, curve?}], at least \
                       one, at most 2048 per call (a lane holds at most 10000 points total). \
                       Values are REAL units unless normalized: true (then 0..=1): volume dB \
                       -60..=6 or \"-inf\", pan -1..=1, mute true/false, a plugin parameter in \
                       its own min..=max or a choice label. curve \"linear\" (default) or \
                       \"stepped\". Bars and beats are 1-based and meter-aware. \
                       \
                       A plugin lane on a FROZEN track is refused (unfreeze first); volume / \
                       pan / mute lanes stay writable. Returns {revision, lane, replaced} — the \
                       lane exactly as it reads back. Undoable, one undo entry. You cannot hear \
                       it: verify with automation_lanes; measure with meter_measure over the \
                       range.",
        annotations(destructive_hint = false, idempotent_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<automation::LaneEditResult>()
    )]
    async fn automation_add_points(
        &self,
        Parameters(params): Parameters<automation::AddPointsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(automation::ADD_POINTS, &params)
            .await
    }

    #[tool(
        description = "Delete points from an automation lane, by indices[] (as automation_lanes \
                       reports them; an out-of-range index is refused, naming the lane's point \
                       count) or by a half-open range {start, end} (bar/beat or sample; points \
                       already inside it), exactly one of the two. \
                       \
                       Target the lane either the usual way — exactly one owner (track_id, \
                       bus_id or master: true) plus control or param — or by lane_id (from \
                       automation_lanes), which is the ONLY way to reach an orphaned lane (a \
                       plugin lane whose instance is gone, from an older project file: it has \
                       no owner to name). Give exactly one of lane_id or a target, never both. \
                       \
                       Deleting every point in the lane removes the lane entirely and requires \
                       confirm: true; without it the call is refused with a summary (point \
                       count and bar range) and the lane is left untouched. A plugin lane on a \
                       FROZEN track is refused (unfreeze first); volume / pan / mute lanes stay \
                       writable. Returns {revision, lane, deleted} normally, or {revision, \
                       lane: null, removed: true, deleted} once the lane is gone. Undoable, one \
                       undo entry. Verify with automation_lanes.",
        annotations(destructive_hint = true, idempotent_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<automation::LaneEditResult>()
    )]
    async fn automation_delete_points(
        &self,
        Parameters(params): Parameters<automation::DeletePointsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(automation::DELETE_POINTS, &params)
            .await
    }

    #[tool(
        description = "Set an automation lane's Read flag (whether it drives its target at \
                       all; its points are kept either way). Declarative: enabled is the flag \
                       to SET, not to toggle. When the lane already has that flag, this is a \
                       no-op ack at the current revision — no undo entry, no revision bump. \
                       Target: exactly one owner (track_id, bus_id or master: true) and exactly \
                       one of control (\"volume\" | \"pan\" | \"mute\") or param (a plugin \
                       parameter). A plugin lane on a FROZEN track is refused (unfreeze first); \
                       volume / pan / mute lanes stay writable. A lane that does not exist is \
                       not_found — automation_set_lane / automation_add_points create one. \
                       Returns {revision, lane}. Verify with automation_lanes.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<automation::LaneEditResult>()
    )]
    async fn automation_set_enabled(
        &self,
        Parameters(params): Parameters<automation::SetEnabledParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(automation::SET_ENABLED, &params)
            .await
    }

    #[tool(
        description = "Delete an automation lane entirely. Target the lane either the usual way \
                       — exactly one owner (track_id, bus_id or master: true) plus control or \
                       param — or by lane_id (from automation_lanes), which is the ONLY way to \
                       reach an orphaned lane (a plugin lane whose instance is gone, from an \
                       older project file: it has no owner to name). Give exactly one of \
                       lane_id or a target, never both. A lane that does not exist is not_found. \
                       \
                       A lane that holds points requires confirm: true; without it the call is \
                       refused with a summary (point count and bar range) and the lane is left \
                       untouched. An empty lane is removed without confirm. Returns {revision, \
                       lane: null, removed: true}. Undoable, one undo entry — edit_undo brings \
                       the lane straight back.",
        annotations(destructive_hint = true, idempotent_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<automation::LaneEditResult>()
    )]
    async fn automation_remove_lane(
        &self,
        Parameters(params): Parameters<automation::RemoveLaneParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(automation::REMOVE_LANE, &params)
            .await
    }
}
