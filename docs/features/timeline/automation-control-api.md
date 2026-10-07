# Automation control API (`automation.*`)

> Verified against master @ `2b28debd` + vocal phase 4 on 2026-10-07.
> Spec history: `docs/history/specs/automation-control-api.md` (built 2026-09-28, merged @ `4c6ef5fb`).

An agent can read, write and shape parameter-automation lanes on a running
Resonance. Examples are "open the filter over bars 17–24", "fade the master
in over bar 1" and "duck the pad on beats 2 and 4". It uses seven control
methods, each with one MCP tool. They drive the same lanes the GUI edits (see
[automation-lanes.md](./automation-lanes.md)). Every write is one undo entry
and one revision bump.

## Behaviour

**Addressing a lane.** Every method takes a flattened `AutomationTargetSpec`:
- exactly one owner: `track_id`, `bus_id` or `master: true`
- exactly one of:
  - `control`: `"volume" | "pan" | "mute"`. Master has volume only.
  - `param`: a plugin parameter by name (case-insensitive), numeric CLAP id or
    first-party string key, the same addressing `track_set_plugin_param` uses.

For plugin lanes, `plugin_id` picks the plugin. If it is omitted, the track's
instrument is used (on a bus or master: the first plugin in the chain).
`occurrence` picks one of several plugins with the same id. `control:
"device"` lanes (external-instrument parameters) are listed but refused as a
write target. `delete_points` and `remove_lane` also accept `lane_id`. That
is the only way to reach an orphaned lane, whose plugin instance is gone.

**Vocal expression curves** (vocal-mcp-control-and-quality.md §10). Instead
of owner + control/param, `vocal: {track_id, curve: "dynamics" | "tension" |
"breathiness" | "pitch_bend"}` (alone) addresses one of a vocal track's four
curve lanes. The track needs a vocal lane in at least one section, else
`invalid_params`.
- A curve is stored once per **section**, in its vocal lane config,
  section-relative. `automation.lanes` lists it at every placement of every
  section with a vocal lane on the track, with `section_relative: true`; a
  section point appears once per placement (in time order, all with
  whole-lane indices).
- Writes take song positions. Each new point is mapped to the placement it
  falls in, then that section and a beat offset, and upserted into the
  section's curve, so every placement follows. A point outside every such
  placement is refused (`invalid_params`, naming its bar and beat), and so
  are two different values for one beat of one section (two placements of
  it). A whole-lane replace keeps a section point while any copy of it is
  still in the list; `delete_points` and `shape` remove every copy of a point
  they remove (`deleted` counts copies). `remove_lane` clears the curve in
  every section (confirm-gated when it has points).
- Vocal lanes always play: `set_enabled` refuses them, and `set_lane` with
  `enabled: false` is refused. They never reach the engine; the curve is
  rendered into the take, so a write leaves `song.vocal` `stale` until
  `vocal.render`. `song.tracks` lane summaries leave them out; `song.vocal`
  lists each lane's `curves: [{curve, points, supported}]`.
- `vocal.set_params` has no `curves` field: the curves are edited only here
  and on the Arrange rows.

**Values.** Values are in real units unless `normalized: true`, which applies
to the whole call:

| Target | Unit | Accepted |
|---|---|---|
| volume | dB | −60..=6, or `"-inf"` (both are silence, read back as `"-inf"`) |
| pan | −1..=1 | number |
| mute | bool | `true`/`false` or 0/1; curve defaults to `stepped` |
| plugin | the plugin's `min..=max` | number or a choice label; stepped params default to `stepped` and round to their steps |
| vocal `dynamics` / `tension` / `breathiness` | `0..=1`, `0.5` = unchanged | number (unit `""`) |
| vocal `pitch_bend` | cents, `-50..=50` | number (unit `"cents"`) |

Out-of-range values are refused, and the error names the range (and the
choices, if any). A parameter without CLAP `IS_AUTOMATABLE` is refused.

**Positions.** A position is `{bar, beat}` (1-based and meter-aware: a 7/8
bar has beats 1–7) or `{sample}`. Positions read back as `SongPosition`
(`bar`, `beat`, `sample`), which can be sent straight back. Bars go through
`check_max_bars`. Points are anchored at a sample. `transport.set_tempo` and
`arrangement.insert/remove_bars` keep a point on its bar. `global.*` tempo
and signature events keep its sample, so its bar moves.

**Methods.**

| Method | What it does |
|---|---|
| `automation.lanes` (read-only) | Lists lanes in owner order: tracks in mixer order, then busses, master, then orphans. It can filter by owner/target and window the points with `range` (indices stay whole-lane). Each lane reports `lane_id`, the target (round-trippable spec + `plugin_name`, `param_id`, `param_name`), `enabled`, `status` (`ok`, `orphaned`, `plugin_missing`, `plugin_initializing`), `unit`, `min`, `max`, `point_count` and points (`index`, `position`, `value`, `normalized`, `text`, `curve`). |
| `automation.set_lane` | Creates the lane or replaces **all** its points. Needs at least one point. Two points on one frame are refused. `enabled` omitted keeps the existing flag; a new lane starts enabled. |
| `automation.add_points` | Inserts points and creates the lane if needed. A point on an occupied frame replaces it (`replaced: n`). Two inputs on one frame are refused. |
| `automation.delete_points` | Takes exactly one of `indices` or a half-open `range`. If the deletion would empty the lane (which removes it), it needs `confirm`. Reports `deleted: n`. |
| `automation.set_enabled` | Sets the Read flag declaratively. If the flag already matches, it acks without a revision bump. |
| `automation.remove_lane` | A lane that holds points needs `confirm`; without it the call is refused with a summary. A missing lane is `not_found`. |
| `automation.shape` | Generates `ramp`, `exp`, `sine`, `triangle`, `square`, `steps` or `random_walk` between `start` and `end`. Points in `[start, end]` are replaced and points outside are kept. Creates the lane if needed. |

Every mutating result is `LaneEditResult { revision, lane, removed?,
replaced?, deleted?, seed? }`. `lane` is the lane as it now reads back, or
`null` once removed.

**`shape` details.** Values are interpolated in real units and then
normalized.
- Points are spaced evenly in ticks **within each bar**, so a 7/8 bar gets as
  many points as a 4/4 bar. The value follows the point's position across the
  whole span, and a partial bar gets its proportional share.
- Default resolutions per bar: `exp` and `sine` 16, `steps` 1, `random_walk` 4.
  `ramp` uses 2 points. `triangle` and `square` use 2 per cycle + 1.
- `exp` needs `from` and `to` > 0 and is refused on volume, pan and mute
  ("dB is already logarithmic; use ramp").
- `random_walk` is a deterministic xorshift from `seed` (default 0), and the
  result echoes the seed.
- The curve always includes a point **at `end`**. For ramp, exp, steps and
  random_walk it holds `to`. For sine, triangle and square it is the
  oscillator's own phase there, which is `from` after whole cycles.

**Limits.** At most 2,048 points written per call (`MAX_POINTS_PER_CALL`,
also `MAX_SHAPE_POINTS_PER_CALL` in the generator) and 10,000 per lane
(`MAX_POINTS_PER_LANE`). A too-dense shape error names the largest resolution
that fits.

**Frozen tracks.** A plugin lane on a frozen track is refused with the
`frozen_reject` error ("unfreeze first"). Volume, pan and mute lanes are
allowed.

**Read-back elsewhere.** In `song.tracks`, `TrackSummary.automation_lanes` is
a count and `TrackDetail.automation` is a `LaneSummary` list with no points.
`master.summary` carries the master's list.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Wire types | `resonance-control/src/methods/automation.rs` | method consts `LANES` … `SHAPE`, `METHODS`, `AutomationTargetSpec`, `LaneControl`, `AutomationValue`, `PointSpec`, `PointView`, `LaneView`, `LaneSummary`, `LaneStatus`, `PointRange`, `*Params`, `LaneEditResult`, `MAX_POINTS_PER_CALL`, `MAX_POINTS_PER_LANE` |
| Curve generator (pure) | `resonance-common/src/automation_shape.rs` | `ShapeKind`, `BarSpan`, `ShapeRequest`, `generate_shape`, `replace_range`, `lands_on_to`, `StepQuantizer` |
| Dispatch | `resonance-app/src/update/control/automation/mod.rs` | `try_handle` |
| Target resolution + frozen rule | `…/automation/target.rs` | `resolve_target`, `resolve_write_target`, `resolve_frozen_checked`, `resolve_lane_or_target`, `target_by_lane_id` |
| Value mapping | `…/automation/value.rs` | real ↔ normalized through `resonance_common` mappings only |
| Shared write path | `…/automation/edit.rs` | `resolve_points`, `current_lane`, `commit_points`, `commit_removal`, `resolve_point_position`, `resolve_range` |
| Views / summaries | `…/automation/view.rs` | `lane_view_for`, `lane_count`, `lane_summaries` |
| Per-method handlers | `…/automation/{lanes,set_lane,add_points,delete_points,set_enabled,remove_lane,shape}.rs` | one fn each |
| Vocal curve target | `…/automation/vocal.rs`, `compose/expression_lanes.rs` | `resolve`, `check_set_lane` (runs `plan_set_lane` before dispatch), `curve_summaries` (`song.vocal`), `VocalCurveTarget`, `VocalCurve`, `LaneView::section_relative`, `VocalCurveSummary` |
| MCP tools | `resonance-mcp/src/tools/automation.rs` | `router_automation` (7 tools), server `INSTRUCTIONS` paragraph in `resonance-mcp/src/server.rs` |

Invariant: each handler computes the lane's complete new point list as a pure
function and dispatches **one** `AutomationMessage` (`SetLane`, `RemoveLane`
or `ToggleRead`) through `update()` via `run_via_update`. That way the gates,
undo classification and revision count behave exactly like a GUI edit. Do not
add a domain message per wire method.

## Surfaces

- **GUI:** none of its own. The same lanes are edited in the mixer inspector
  and on the Arrange timeline ([automation-lanes.md](./automation-lanes.md)).
- **Control API / MCP:**
  - `automation.lanes` → `mcp__resonance__automation_lanes`
  - `automation.set_lane` → `mcp__resonance__automation_set_lane`
  - `automation.add_points` → `mcp__resonance__automation_add_points`
  - `automation.delete_points` → `mcp__resonance__automation_delete_points`
  - `automation.set_enabled` → `mcp__resonance__automation_set_enabled`
  - `automation.remove_lane` → `mcp__resonance__automation_remove_lane`
  - `automation.shape` → `mcp__resonance__automation_shape`
  - Lane summaries also appear in `song_tracks` and `master_summary`.
- **Agent skills:** `resonance-agent-plugin/skills/mixing/SKILL.md` (fades and
  level rides via `automation_set_lane`, read back with `automation_lanes`).
  `skills/mastering/SKILL.md` (an `automation_shape` on a master plugin
  parameter).

## How it's tested

| What | Where | Run |
|---|---|---|
| Shape generator: counts, D3 end point, range replace, exp rejection, random-walk bounds and determinism | `resonance-common/tests/automation_shape.rs` | `./scripts/run-tests.py -p resonance-common` |
| `lanes` / `set_lane`: round-trip, −inf, stepped labels, normalized, undo/redo, errors, frozen (control and GUI path), 7/8 positions, set_tempo vs global tempo event, filter/order/window, orphans, `song_tracks`/`master_summary`, `hello` | `resonance-app/tests/control/control_automation.rs` | `./scripts/run-tests.py -p resonance-app` |
| `add_points` / `delete_points` / `set_enabled` / `remove_lane`: upsert, confirm gating, `lane_id` for orphans, undo/redo, no-op ack | `resonance-app/tests/control/control_automation_edit.rs` | same |
| `shape` through the app: 7/8 density, worked cutoff example, limits, one revision | `resonance-app/tests/control/control_automation_shape.rs` | same |
| Render: the lane `set_lane` sends is the one the engine renders louder; measure/stems windows | `resonance-app/tests/control/control_automation_render.rs` + `resonance-audio/tests/bounce/automation_render.rs` | `-p resonance-app`, `-p resonance-audio` |
| Vocal curves: listing, placement mapping, outside / conflict refusals, copy deletion, set/remove, no Read flag, `set_params` refuses `curves`, stale without render, GUI debounce, engine never sent, save/reopen + unrelated undo, one undo entry | `resonance-app/tests/control/control_vocal_curves.rs` | same |
| Every method has a tool; schemas and descriptions | `resonance-mcp/tests/schema_hygiene.rs`, `doc_hygiene.rs`, `translation.rs`; skills naming tools: `agent_plugin_lockstep.rs` | `./scripts/run-tests.py -p resonance-mcp` |

Not covered: no test drives the MCP tools against a live app end to end. The
render proof is split: the app test checks the dispatched lane and window,
and the audio test checks that this exact lane renders louder.

## Decisions and deviations

From the spec, as merged (`4c6ef5fb`):
- **D1** (prune lanes on plugin remove or swap), **D2** (refuse plugin lanes on
  frozen tracks; the GUI gate added too), **D3** (`shape` writes a point at
  `end`) and **D4** (`add_points` upserts an occupied frame) were built as
  recommended.
- **Deviation:** `shape` lands on `to` only for ramp, exp, steps and
  random_walk. Oscillators end on their own phase, so a triangle keeps its
  down leg and there is no spike at the last step.
- **Deviation:** shape values follow tick position across the whole span, and
  partial bars get a proportional share of points. The spec only said "evenly
  per bar".
- **Deviation:** `delete_points` and `remove_lane` accept `lane_id`, which the
  spec did not have, so orphaned lanes from older files can be removed.
- **Deviation:** the audible proof lives in `resonance-audio/tests/bounce`
  instead of a single app-level meter test.
- Slice A0 made `meter.measure`, `meter.stems` and stem export play
  automation, like `render.mixdown` (`resonance-audio/src/engine/bounce/stem.rs`
  module doc).
- Adding the namespace did not bump `PROTOCOL_VERSION`, because the methods
  were added only. The vocal target (`vocal`, `section_relative`, the
  `song.vocal` `curves` list) is additive too: no bump.

## Known gaps

- Device-param lanes are read-only here. There are no tools for them.
- The `text` of a plugin point is the choice label or `"<value> <unit>"`, not
  the plugin's own `value_to_text`.
- There is no tempo automation and no curve kinds beyond linear and stepped.
  `shape` densifies instead.
- Recording automation from live moves is not built. See
  [../../intents/timeline/automation-recording.md](../../intents/timeline/automation-recording.md).
