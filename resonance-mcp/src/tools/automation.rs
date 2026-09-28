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

    #[tool(
        description = "Generate an automation curve over a range — a sweep, fade, ride or LFO \
                       — and write it into the lane (created if missing; undoable, one undo \
                       entry). Target as in automation_set_lane: exactly one owner (track_id, \
                       bus_id or master: true) and exactly one of control (\"volume\" | \"pan\" \
                       | \"mute\") or param (plugin_id omitted = the track's instrument). \
                       \
                       start / end: {bar, beat} | {sample}; bars and beats are 1-based and \
                       meter-aware. The curve starts at start holding from and INCLUDES a point \
                       AT end holding to, so the value arrives: {bar: 25} as end sweeps over \
                       bars 17-24 and lands on the downbeat of bar 25. Only points in [start, \
                       end] (both ends included) are replaced; points outside are kept (unlike \
                       automation_set_lane, which replaces ALL points). \
                       \
                       shape: \"ramp\" (2 points, the engine interpolates exactly), \"exp\" \
                       (geometric from -> to, both > 0; for a plugin parameter such as a \
                       cutoff in Hz, where lanes are linear in plain units; refused for \
                       volume / pan / mute — dB is already logarithmic, use ramp), \"sine\" \
                       and \"triangle\" (from <-> to, cycles times, default 1), \"square\" \
                       (from / to, stepped), \"steps\" (a stair from -> to, stepped; resolution \
                       = steps per bar), \"random_walk\" (bounded to from..to, starts at from, \
                       deterministic from seed, default 0; the seed used is echoed). \
                       resolution = points per bar (defaults: exp / sine 16, steps 1, \
                       random_walk 4), spaced evenly within each bar, so a 7/8 bar gets as \
                       many points as a 4/4 bar, while the curve's value follows time. At most \
                       2048 points per call; past it the error names the largest resolution \
                       that fits. Mute and stepped plugin parameters are always stepped and \
                       rounded to their steps. \
                       \
                       Values are REAL units unless normalized: true (then 0..=1 for the whole \
                       call): volume dB -60..=6 or \"-inf\" (silence), pan -1..=1, mute \
                       true/false, a plugin parameter in its own min..=max or a choice label. \
                       \
                       TEMPO / METER: points are anchored at a sample. transport_set_tempo and \
                       arrangement_insert/remove_bars keep a point at its bar; global_* tempo \
                       and signature events keep its sample, so its bar moves. A plugin lane \
                       on a FROZEN track is refused (unfreeze first). \
                       \
                       Example — open the wavetable instrument's filter over bars 17-24: \
                       {\"track_id\": 3, \"param\": \"Filter Cutoff\", \"start\": {\"bar\": \
                       17}, \"end\": {\"bar\": 25}, \"shape\": \"exp\", \"from\": 300, \"to\": \
                       4000} sweeps the cutoff exponentially from 300 Hz at bar 17 to 4 kHz at \
                       bar 25 (the end of bar 24). Then automation_lanes to read it back, and \
                       meter_measure over bars 17-24 in two halves (17-20, 21-24) to see the \
                       spectral / level change. \
                       \
                       Returns {revision, lane, seed?} — the whole lane as it reads back. You \
                       cannot hear it: verify with automation_lanes; measure with meter_measure \
                       over the range.",
        annotations(destructive_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<automation::LaneEditResult>()
    )]
    async fn automation_shape(
        &self,
        Parameters(params): Parameters<automation::ShapeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(automation::SHAPE, &params).await
    }
}
