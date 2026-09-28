# automation.* — parameter automation on the control API and MCP

Status: **built** (2026-09-28, branch automation-api; see the merge commit for deviations). Build it as vertical slices,
one tool per control method. Each slice lands its wire type, app handler,
MCP tool and tests together.

## 0. Why

Automation already works in the model, the engine, save/load, undo and the
GUI. The control surface has no way to create it or read it, so an MCP agent
cannot write "open the filter over bars 17–24, then close it". Agents fake
this with LFOs and velocity routes today. This plan adds an `automation.*`
namespace plus MCP tools, and fixes the gaps that reading the code turned
up (§2).

## 1. What already exists (verified against master @ f95f2afe)

| Piece | Where | Relevant facts |
|---|---|---|
| Model | `resonance-common/src/automation.rs` | **One lane per `AutomationTarget`** (app and engine both key by target). `Breakpoint.value` is normalized 0..1. `Breakpoint::new` clamps. `sort_points` is stable, so equal-frame duplicates are *allowed* by the model. Curves: `Linear`, `Stepped`. Gain maps to −60..+6 dB, pan to −1..1, mute thresholds at 0.5. `PluginParam` is **linear in the plugin's own min..max** (`lane_value_to_plugin_param`). |
| App edits | `resonance-app/src/update/automation.rs` | `AutomationMessage::{AddLane, RemoveLane, ToggleRead, AddBreakpoint, DeleteBreakpoint, SetCurveKind, *Drag*}`. Each is `Record` except the drag bracket. **Deleting the last point removes the lane** (an empty enabled lane samples to the floor). There is no whole-lane replace message. |
| App mirror | `resonance-app/src/state/automation.rs` | `AutomationState.lanes: HashMap<AutomationTarget, AutomationLane>`. The app allocates lane ids. |
| Undo | `undo/snapshot.rs::restore_automation_lanes` | The snapshot carries every lane and restore reconciles the engine. `control::execute` already wraps each mutating call in `with_compound_undo`, which gives **one call → one undo entry → one revision**, however many messages it dispatches. |
| Engine | `resonance-audio/src/engine/automation.rs`, `mixer/automation_apply.rs` | A gain lane at the floor (−60 dB) renders **exact silence**, so "−∞" is representable. A plugin lane whose instance is missing is dropped from the snapshot, which makes it inert. |
| Tempo | `update/tempo_reanchor.rs` | **Only `transport.set_tempo` (CommitBpm) and `arrangement.insert/remove_bars` re-anchor automation.** `global.*` tempo/meter events do not re-anchor anything: clips, markers and automation all keep their sample position. |
| Positions | `control/transport.rs::resolve_position`, `view_model/position.rs::song_position` | These are the one resolver and the one projection for `{bar, beat}` ↔ sample. They are meter-aware, and beat counts in the signature's beat unit. |
| Plugin addressing | `control/track/params.rs` | `find_param` matches by name (case-insensitive), then numeric id, then stable key. `ParamValue::resolve` handles choice labels. `clamp_within_tolerance` absorbs f32 widening. The busy error covers a plugin that is still initializing. `frozen_reject` refuses plugin-param edits on frozen tracks. |

## 2. Gaps found while reading. They change the scope.

1. **`meter.measure` and `meter.stems` ignore automation.**
   `engine/bounce/stem.rs:592` renders with `AutomationSnapshot::default()`,
   and `measure.rs` goes through `render_stem`. `render.mixdown` plays
   automation because the export path threads `ctx.automation.load_full()`.
   The module doc at `stem.rs:34` claims the opposite and is stale. Fix: in
   `thread/dispatch/bounce.rs`, pass `ctx.automation.load_full()` into
   `measure_mix_spawn` and `export_stems_spawn`, and on through
   `render_stem`. This is an engine change and has to land first (slice
   A0). It changes what `meter.*` reports for any project that has lanes,
   and that is the intended fix.
2. **Removing or swapping a plugin orphans its lanes.**
   `engine_events/plugins.rs::track_removed` (and its bus/master siblings)
   prunes the slot, the state cache and the key route, but **not the
   automation lanes**. Track deletion does prune them
   (`engine_events/tracks.rs:187`). An orphaned lane is inert but still
   saved. The comment in `track_removed` (ba todo #1311) records that the
   same leak in sidechain routes let them reattach to whichever plugin
   later reused the id. A swap in `plugin_replace` issues a fresh id, so
   the swapped-out plugin's lanes are orphaned too. A restore of a missing
   plugin keeps its id, and its lanes survive, which is correct.
   → Decision D1.
3. **Automation edits are not frozen-input edits.** `frozen_input_edit_target`
   does not list `Message::Automation`, so the GUI lets you edit a
   plugin-param lane on a frozen track. The edit is inaudible because the
   frozen cache plays. It only surfaces as a fingerprint mismatch on the
   next load (`freeze.rs::freeze_content_fingerprint` hashes the enabled
   plugin-param lanes). `track.set_plugin_param` rejects the same edit.
   → Decision D2.
4. **Plugin lanes are linear in plain units.** For a skewed parameter such
   as the wavetable's `Filter Cutoff`, 20..20000 Hz with skew −2.5, two
   points at 300 Hz and 4 kHz sweep linearly *in Hz*. A perceptual
   (exponential) sweep therefore needs density, and `automation.shape`
   provides it (§4.6). Nothing changes in the engine for this.
5. **The plugin's `text` rendering is only known for the current value.**
   There is no synchronous `value_to_text` app-side. So `automation.lanes`
   reports `text` as the choice label for stepped params, and as
   `"<value> <unit>"` otherwise. It does not make an FFI round trip per
   point.

## 3. Decisions to confirm before building

- **D1: orphaned lanes (recommended: prune + report).** Prune a plugin's
  lanes in `track_removed`, `bus_removed` and `master_removed`, and on a
  swap in `plugin_replace`, the way track deletion already does. The
  removal is already a snapshot-undoable edit, so undo brings the lanes
  back. `automation.lanes` still reports any orphan it finds (from older
  project files) with `status: "orphaned"`. *Alternative:* keep the lanes
  and only report them. That is safe only if instance ids can never be
  reused across a save/load, and the #1311 comment says they can.
- **D2: frozen tracks (recommended: reject plugin lanes, allow mixer
  lanes).** Plugin-param lanes on a frozen track get the same
  `frozen_reject` error `set_plugin_param` gives ("unfreeze first"). Gain,
  pan and mute lanes stay allowed, because the mixer stays live while
  frozen. Also add `Message::Automation` plugin-param edits to
  `frozen_input_edit_target`, so the GUI stops making the same silent
  edit.
- **D3: `shape` writes its end point.** The task text says [start, end).
  Recommended instead: the generated curve includes a point **at `end`**
  holding `to`, so the value actually arrives. Existing points in
  **[start, end]** are replaced and points outside are kept. Under the
  literal reading, a ramp would stop one step short and whatever point
  sits after `end` would take over.
- **D4: `add_points` at an occupied frame upserts** the existing point
  (the `global.add_*` precedent) and reports `replaced: n`. Within one
  `set_lane` / `add_points` call, two inputs that resolve to the same
  frame are rejected, naming both indices.

## 4. Wire design (`resonance-control/src/methods/automation.rs`)

A new namespace `automation`, added to `methods::capabilities()`. It only
adds methods, so there is **no `PROTOCOL_VERSION` bump**, per the rule in
`lib.rs`, and `lockstep.json` stays at 1.

### 4.1 Target

```jsonc
// exactly one owner:
{ "track_id": 3 } | { "bus_id": 7 } | { "master": true }
// then exactly one of:
"control": "volume" | "pan" | "mute"          // mixer lanes (master: volume only)
"param": "Filter Cutoff" | "412" | "lim_on"   // plugin lanes
// plugin lanes only:
"plugin_id": "com.resonance.wavetable",        // omitted = the track's instrument
"occurrence": 0
```

`control` and `param` are separate fields. A plugin whose parameter is
called "Volume" or "Pan" would otherwise be ambiguous with the fader.
Resolution goes through the existing `view_model::plugin_entries`,
`find_param` and `instance_for` (lift the bus and master equivalents
into a shared `resolve_plugin_target`), so the error wording is shared
too:
- unknown track or bus → `no_track` / `no_bus`
- missing instrument → the plugins the track carries
- wrong plugin → the `unknown_plugin_on_track` text
- wrong param → `has no parameter "X" (has: [...])`
- empty param list → the existing *still initializing* `busy`

`DeviceParam` lanes are listed read-only (`control: "device"`,
`param: <id>`) and cannot be set here.

### 4.2 Values

A value is **in the target's real units unless `normalized: true`**, which
applies to the whole call.

| Target | Real unit | Accepted | Notes |
|---|---|---|---|
| volume | dB | −60..=+6, or `"-inf"` | −60 and `"-inf"` both map to normalized 0, which the engine renders as silence. Read-back reports `"-inf"` there. |
| pan | −1..=1 | number | |
| mute | bool | `true`/`false` (also 0/1) | Default curve `stepped`. |
| plugin | plugin min..=max | number or choice label | Uses `resolve_param_value`: labels, finiteness and f32 tolerance. Stepped params default to curve `stepped`. |

Out-of-range values are rejected with the range, and with the choices
where there are any. Conversion goes only through `real_to_lane_value`,
`lane_value_to_real`, `plugin_param_to_lane_value` and
`lane_value_to_plugin_param`. Nothing is duplicated.

### 4.3 Point and lane views

```jsonc
// input point
{ "position": {"bar":17,"beat":1} | {"sample":123}, "value": 300.0, "curve": "linear"|"stepped" }
// reported point
{ "index":0, "position":{"bar":17,"beat":1.0,"sample":1632000},
  "value":300.0, "normalized":0.01402, "text":"300 Hz", "curve":"linear" }
// lane view
{ "lane_id":4, "target":{ …as in 4.1, plus "plugin_name":"Wavetable" },
  "enabled":true, "status":"ok"|"orphaned"|"plugin_missing"|"plugin_initializing",
  "unit":"Hz", "min":20.0, "max":20000.0, "points":[…] }
```

Every mutating result is `{revision, lane}`: the lane exactly as it
reads back. An agent gets the resolved positions without a second call.
After `remove_lane`, or a `delete_points` that empties the lane, the
result is `{revision, lane: null, removed: true}`.

### 4.4 Methods

| Method | Params | Semantics | Dispatches |
|---|---|---|---|
| `automation.lanes` (read-only) | optional owner filter + optional `plugin_id`/`occurrence`/`param`/`control`; `range?: {start,end}` to window the points | Lists lanes in `(owner, target_priority, param)` order. Orphans are included with `status` set. | none |
| `automation.set_lane` | target, `points[]` (≥1), `normalized?`, `enabled?` | Creates or **replaces all points**, sorted. A duplicate frame is rejected. Empty `points` is rejected ("use remove_lane"). | new `SetLane` |
| `automation.add_points` | target, `points[]`, `normalized?` | Inserts; an occupied frame upserts (D4). Creates the lane if missing. | `SetLane` |
| `automation.delete_points` | target, **either** `indices[]` **or** `range{start,end}` (half-open) | Deleting every point removes the lane; that needs `confirm`. Reports `deleted: n`. | `SetLane` / `RemoveLane` |
| `automation.set_enabled` | target, `enabled: bool` | Declarative. Dispatches `ToggleRead` only when the flag differs; otherwise it is an ack with no revision bump. | `ToggleRead` |
| `automation.remove_lane` | target, `confirm?` | Refused with a summary ("lane has 12 points, bars 17–24") unless `confirm`, when the lane has points. A missing lane is `not_found`. | `RemoveLane` |
| `automation.shape` | target, `start`, `end`, `shape`, `from`, `to`, `cycles?`, `resolution?`, `seed?`, `normalized?` | Generates §4.6. Replaces points in [start, end] (D3) and keeps the rest. Creates the lane if missing. | `SetLane` |

**New domain message:** `AutomationMessage::SetLane { target, points,
enabled }`, classified `Record` and described as "automation edit". It
creates the lane (allocating an id) or replaces it wholesale via
`store_lane`. The handlers compute the new point list as a pure function
and dispatch **one** `SetLane`, so the mutation goes through `update()`,
passes the gates and is undoable like any GUI edit. Do not add a
separate message per wire method.

Limits: **≤ 2,048 points written per call**, **≤ 10,000 points per
lane**. Past either limit the call is rejected with the count and the
limit, and the error suggests a lower `resolution`. Bars go through the
shared `check_max_bars` guard.

### 4.5 Integration

- **`song.tracks` / `song.summary`**: `TrackSummary` gains
  `automation_lanes: usize`. `TrackDetail` gains `automation: [{control|param,
  plugin_id?, occurrence?, points, enabled, status}]`, a compact list with
  no points. `master.summary` gets the same list for master lanes. A plugin
  lane is listed under the track or bus that owns the instance.
- **Tempo/meter**: behaviour stays as it is, documented and tested:
  - `transport.set_tempo`: a lane at bar 17 **stays at bar 17**
    (re-anchored).
  - `global.add/edit/remove_tempo_event` and every signature edit: lanes
    **keep their sample position**, like clips and markers, so their bar
    position moves.
  - `arrangement.insert/remove_bars`: points after the cut shift (this
    exists already).

  All of this goes into the `automation_lanes` tool description.
- **Render**: slice A0 (§2.1). After it, `render.mixdown`, `meter.measure`
  and `meter.stems` all play automation.
- **Plugin removal/replace**: D1. **Frozen**: D2.

### 4.6 `automation.shape`

Every shape interpolates in the **target's real units**, then normalizes.
Points are spaced evenly **in ticks within each bar**, so a 7/8 bar gets
the same number as a 4/4 bar, and they are resolved through the tempo map.

| shape | Curve | Default resolution | Points |
|---|---|---|---|
| `ramp` | linear | — | 2 (the engine interpolates exactly) |
| `exp` | geometric from→to | 16 / bar | Requires `from`,`to` > 0. Rejected for volume, pan and mute, with "dB is already logarithmic; use ramp". |
| `sine` | from↔to, `cycles` (default 1) | 16 / bar | |
| `triangle` | from↔to | — | 2 per cycle + 1 (exact corners) |
| `square` | from/to, stepped | — | 2 per cycle + 1 |
| `steps` | stair from→to, stepped | 1 / bar | `resolution` = steps per bar |
| `random_walk` | bounded to [min(from,to), max(from,to)], starts at `from` | 4 / bar | deterministic xorshift from `seed` (default 0); the seed used is echoed |

`resolution` is capped by the per-call limit, and the error names the
largest resolution that would fit. Mute and stepped plugin params force
the curve to `stepped` and round values to the nearest step.

## 5. MCP (`resonance-mcp/src/tools/automation.rs`)

A new `router_automation()`, added to `combined_router()`. It has seven
tools: `automation_lanes`, `automation_set_lane`, `automation_add_points`,
`automation_delete_points`, `automation_set_enabled`,
`automation_remove_lane` and `automation_shape`. They use `invoke_structured`
with `output_schema` from the wire types, so the schemas cannot drift.
`schema_hygiene`, `doc_hygiene` and
`combined_router_exposes_every_control_method` enforce the rest.

Every description must state:
- values are REAL units unless `normalized: true`, with the unit table in
  one line;
- bars and beats are 1-based and meter-aware;
- `set_lane` replaces ALL points, and `shape` replaces only [start, end];
- the tempo/meter behaviour;
- "verify with automation_lanes; measure with meter_measure over the
  range".

`automation_shape` carries this worked example:

```json
{"track_id": 3, "param": "Filter Cutoff", "start": {"bar": 17}, "end": {"bar": 25},
 "shape": "exp", "from": 300, "to": 4000}
```

It sweeps the wavetable instrument's cutoff (no `plugin_id` needed)
exponentially from 300 Hz at bar 17 to 4 kHz at bar 25, which is the end
of bar 24. The example also says: then `automation_lanes` to read it back,
and `meter_measure` over bars 17–24 in two halves to see the
spectral/level change.

Add one paragraph to `INSTRUCTIONS` in `server.rs`: automation exists, it
is per-target, which tools read it, and "an agent can't hear it —
measure". The resonance-agent-plugin `mixing` skill can mention
`automation_shape` for fades and rides. That is optional, and the
lockstep test will check the tool names if it does.

## 6. Slices (each is a vertical slice with wire, handler, tool and tests)

| # | Slice | Contents |
|---|---|---|
| A0 | Engine: automation in measure/stems | §2.1, plus a `resonance-audio/tests/bounce` test: a master-gain lane ramps a sine source, and `measure_mix` sees a rising level. Fix the stale `stem.rs` doc. Run with `RESONANCE_RENDER_THREADS=8`. |
| A1 | D1 prune on plugin removal/swap | Tests in `tests/mixer`: remove an effect → lane gone; undo → lane back; swap → lanes gone; restore of a missing plugin → lanes kept. |
| A2 | `automation.lanes` + wire module + target/value resolution + `song.tracks` counts | Read-only and safe to land first. It establishes the target resolver and value mapping the rest reuse. |
| A3 | `SetLane` message + `automation.set_lane` | The main tool. Includes D2. |
| A4 | `add_points`, `delete_points` | |
| A5 | `set_enabled`, `remove_lane` | confirm gating |
| A6 | `automation.shape` | Put the generator as a pure fn in `resonance-common/src/automation_shape.rs` so it is unit-testable from `resonance-common/tests/`. |
| A7 | Docs | MCP `INSTRUCTIONS`, tool list, ARCHITECTURE.md if the layering table needs it (it should not: no new crate). |

## 7. Tests

Follow CLAUDE.md: `Resonance::new_for_test()`. Put new modules in the
existing group binaries and add **no** new top-level files. There are no
inline `#[cfg(test)]` modules.

- **Unit** (`resonance-common/tests/automation.rs`, and a new
  `automation_shape.rs`):
  - value round-trips for dB (incl. −60 ⇄ `-inf`), pan, plugin min/max
    (20..20000), and stepped labels ⇄ index;
  - shape point counts per shape and resolution;
  - range-replace keeps outside points and replaces those at the edges
    (D3);
  - `exp` rejection on dB;
  - `random_walk` stays within bounds and is deterministic per seed;
  - limit errors.
- **Position** (`tests/control`): add 7/8 at bar 17
  (`global.add_signature_event`), `set_lane` at `{bar:17}` and
  `{bar:18, beat:7.5}`, then read back → the same bar/beat and the
  expected samples. A beat of 8 in bar 17 is refused.
- **Control** (`tests/control/control_automation*.rs`, registered in
  `tests/control.rs`):
  - `set_lane` → `lanes` round-trip equality (positions, real values,
    normalized, curve);
  - undo → the previous lane *exactly* (including "no lane"); redo → the
    new one;
  - one call bumps the revision by exactly one;
  - `remove_lane` / emptying `delete_points` without `confirm` is refused
    and leaves the lane intact;
  - a bad param name error lists the valid names;
  - unknown track or bus → `not_found`;
  - an out-of-range value gives an error naming the range;
  - a frozen track rejects plugin lanes and accepts volume lanes (D2);
  - tempo: `transport.set_tempo` keeps bar 17, and
    `global.add_tempo_event` at bar 9 keeps the sample.
- **Render** (`tests/control/control_meter` or a sibling): a master volume
  lane going −inf → 0 dB over bars 1–4 on a sustained source.
  `meter.measure` over bar 1 and over bar 4 → `lufs_momentary` (or band
  energy) at bar 4 is > bar 1 + 20 dB. Guard it against silence at bar 4,
  per [silent goldens are vacuous]. Do the same through `meter.stems`.
- **Capability**: every `automation::METHODS` entry is in `control.hello`
  and has an MCP tool, via the existing
  `combined_router_exposes_every_control_method`. Add an explicit assert
  on `hello` in the control tests anyway.

## 8. Out of scope

- Recording automation from live control moves.
- New tools for `DeviceParam` lanes (they are listed read-only).
- Tempo automation.
- A plugin `value_to_text` round trip per point (§2.5).
- Curve kinds beyond linear and stepped. `shape` densifies instead.
