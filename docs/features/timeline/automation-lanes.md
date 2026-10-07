# Automation lanes

> Verified against master @ `2b28debd` + vocal phase 4 on 2026-10-07.
> Spec history: none for the model and GUI (built under epic #14, arch doc #162; lane rows doc #256). The control API has `docs/history/specs/automation-control-api.md`, see [automation-control-api.md](./automation-control-api.md).

A track's volume, pan or mute, a bus's volume, pan or mute, the master
volume, any automatable plugin parameter, or an external instrument's device
parameter can follow a breakpoint curve over the song. Lanes are added in
the mixer inspector, drawn and edited on the Arrange timeline, played by the
engine in live playback and in every offline render, and saved with the
project.

## Behaviour

**Model.** One lane per target (`AutomationTarget`). A lane has an id, a Read
flag (`enabled`) and a time-sorted list of `Breakpoint { time_frames, value,
curve }`. `value` is always normalized `0..=1`; the real value depends on the
target:

| Target | Real range |
|---|---|
| `TrackGain` / `BusGain` / `MasterGain` | −60..+6 dB, linear in dB. The floor renders as exact silence, not −60 dBFS. |
| `TrackPan` / `BusPan` | −1..1 (same stereo-balance law as the pan knob) |
| `TrackMute` / `BusMute` | off / on, threshold at 0.5 |
| `PluginParam { instance, param_id }` | the plugin's own `min..=max`, **linear in plain units** |
| `DeviceParam { track, param_id }` | the device definition's binding range (CC / NRPN), mapped by the engine |
| `VocalCurve { track, curve }` | **render-only**: `0..=1` for Dynamics / Tension / Breathiness, `-50..=50` cents for Pitch bend (`VocalCurveKind::value_range`) |

Curves are `Linear` (interpolate to the next point) or `Stepped` (hold).
Before the first point and after the last, the lane holds that point's value.
There is no master pan or mute lane, and no tempo automation.

**Vocal expression curves** (vocal-mcp-control-and-quality.md §10).
`AutomationTarget::VocalCurve` is not stored as a lane: the curves live in
each section's vocal lane config (`VocalParams::curves`, section-relative
beats) and are saved and undone with it. `AutomationState::vocal`
(`compose::expression_lanes::VocalLanes`) projects them onto the timeline —
for every track with a vocal lane in any section, one `VocalLane` per curve:
an `AutomationLane` with a derived id (`vocal_lane_id`, ≥ `1 << 62`) whose
points are each section point at each placement, the `PointOrigin` of every
projected point, and a `CurveWindow` per placement (frames, section beat 0,
length, whether the bank renders the curve, the bank's name). It is rebuilt
after every outermost update (`update::compose::expression::
refresh_vocal_lanes`) and at the start of every `automation.*` call. It is
never persisted, never in the undo snapshot and never sent to the engine:
`AutomationTarget::is_engine_lane` is false for it, `store_lane`, project
save and the undo restore filter on it, and the engine drops such a
`SetAutomationLane` too. Every `AutomationMessage` on a `VocalCurve` target
goes to `update::compose::expression::handle`, which maps the frame to its
placement → section → beat and edits that section's curve (add, drag within
the placement, delete, curve kind, whole-lane replace via `plan_set_lane`,
remove = clear every section). How the render uses the curves is in
[vocal rendering](../compose/vocal-rendering.md).

**Lifetime rules.**
- A new lane is seeded with one point at frame 0 holding the target's current
  static value, so turning it on does not jump the parameter.
- Deleting the last point removes the lane. An enabled empty lane would pin the
  target to its floor.
- Read off keeps the points; the target falls back to its static value.
- A plugin whose CLAP `IS_AUTOMATABLE` flag is unset cannot get a new lane
  (`lane_allowed`). A lane that already exists stays editable.
- Removing a plugin from a chain, or swapping it for another, drops every lane
  aimed at its instance (`drop_plugin_lanes`). Deleting a track drops its own
  lanes and its plugins' lanes. Undo brings them back.
- On a frozen track a plugin-param lane edit counts as an edit to the freeze's
  inputs: the edit is dropped and the freeze turns stale. Gain, pan and mute
  lanes stay editable, because the mixer stays live while frozen.

**Positions and tempo.** Points are stored in sample frames, not ticks.
Changing the song tempo (`CommitBpm`) and inserting or removing bars re-anchor
every point to its musical position (`update/tempo_reanchor.rs`). Points
inside a removed span are not deleted. They stay put and the lane is
re-sorted. Editing a tempo or signature event on the global track does not
re-anchor anything, so a point keeps its sample and its bar moves. Moving a
clip does not move automation: lanes belong to the track, not to clips.

**Arrange view.**
- A track with lanes shows a "▸ N auto" caret in its header.
- Collapsed: the track row draws one lane as an overlay band inside the clip
  row, inset 18 px top and bottom. Which lane it draws is the "shown" lane:
  gain, pan, mute, then device params by id, then plugin params by param id.
  With more than one lane, a label chip on the band cycles to the next lane
  (`CycleTrackLane`). The cycle choice is transient view state.
- Expanded (caret click, `ToggleTrackExpanded`): every lane of the track gets
  its own 44 px sub-row under the track. The header column draws a matching
  indented label cell (`view_automation_lane_header`), and the overlay band is
  suppressed. Expansion is transient and not saved.
- Only lanes that belong to an Arrange track show here. Bus and master lanes
  are edited only in the mixer.
- A vocal track (one with a vocal lane in some section) counts its four
  curve rows in "N auto"; expanded, it gets `Vocal: Dynamics | Tension |
  Breathiness | Pitch bend` sub-rows after its engine lanes (same
  `ArrangeRowKind::AutomationLane`, derived lane ids). The row is dimmed
  except at placements of sections with a vocal lane, which are raised
  bands carrying that section's curve; editing at any placement edits the
  section, and every placement follows. Outside placements the row is
  inert. A placement whose bank does not render the curve (Tension on
  TIGER, Breathiness on every shipped bank) is greyed with "not supported by
  <bank>" and ignores clicks. Vocal rows never join the collapsed overlay,
  its chip cycle, or the live playhead value.

**Editing gestures (Arrange canvas).**
- Click on the bare band (not on a clip or a dot): add a point, snapped to the
  grid. The value comes from the click height.
- Drag a dot: move it. Unsnapped, and clamped between its neighbours in time.
  The whole drag is one undo entry.
- Double-click a dot: toggle linear ↔ stepped.
- Right-click a dot, or Delete after clicking it: delete the point.
- Hit order is: lane chip, take-lane comp ribbon, breakpoint dots, clips, then
  band-add. Dots use a 7 px pick radius.

**Mixer.** The inspector's AUTOMATION group lists every lane on the channel
with a Read toggle and a remove button, and has the `+ Add lane` picker. This
is the only GUI place a lane is created, removed or switched on and off. While
playback reads a lane, the fader or pan knob is tinted with the live value.
The engine sends this throttled (`AutomatedValue`, at most every 30 ms per
target, only on moves ≥ 1/512).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model | `resonance-common/src/automation.rs` | `AutomationTarget`, `AutomationLane`, `Breakpoint`, `CurveKind`, `sample_lane`, `lane_value_to_real` / `real_to_lane_value`, `lane_value_to_plugin_param` / `plugin_param_to_lane_value`, `GAIN_MIN_DB` |
| App mirror | `resonance-app/src/state/automation.rs` | `AutomationState { lanes, live_values, lane_selection }`, `alloc_lane_id`, `target_priority`, `target_belongs_to_track`, `track_lanes_sorted` |
| Edits | `resonance-app/src/update/automation.rs` | `AutomationMessage` (`AddLane`, `RemoveLane`, `ToggleRead`, `AddBreakpoint`, `DeleteBreakpoint`, `SetLane`, `SetCurveKind`, drag trio, `CycleTrackLane`, `ToggleTrackExpanded`), `undo_action`, `edited_target`, `lane_allowed`, `store_lane` |
| Engine echo → mirror | `resonance-app/src/engine_events/automation.rs` | `AutomationLaneChanged`, `AutomationLaneCleared`, `AutomatedValue` |
| Plugin / track prune | `resonance-app/src/engine_events/plugins.rs`, `engine_events/tracks.rs`, `update/plugin_replace.rs` | `drop_plugin_lanes`, `drop_track_references` |
| Frozen gate | `resonance-app/src/update/gates.rs` | `frozen_input_edit_target` (`Message::Automation` arm) |
| Tempo / bars | `resonance-app/src/update/tempo_reanchor.rs`, `update/arrangement.rs` | `musical_anchors`, `reanchor_to_tempo`, `resort_automation` |
| Undo / restore | `resonance-app/src/undo/snapshot.rs` | `restore_automation_lanes` |
| Persistence | `resonance-app/src/project/model.rs`, `update/project_io/serialize.rs`, `update/project_io/reconcile/restored.rs` | `automation_lanes` field (sorted by lane id), `restored::AutomationLanes` (Tail stage, after external instruments) |
| Arrange drawing / input | `resonance-app/src/view/timeline/automation/{geometry,draw,input,vocal}.rs`, `view/timeline/input/{pointer,hover,keyboard}.rs` | `automation_band`, `lane_row_band`, `shown_lane_for_track`, `breakpoint_hit`, `band_add_at`, `breakpoint_drag_to`, `BREAKPOINT_HIT_RADIUS`, `draw_vocal_curve_row`, `vocal_rows_hash` |
| Vocal curve rows | `resonance-app/src/compose/{expression,expression_lanes}.rs`, `update/compose/expression.rs` | `VocalLanes`, `VocalLane`, `CurveWindow`, `PointOrigin`, `project_vocal_lanes`, `plan_set_lane`, `vocal_lane_id`, `sample_at_beat`, `curve_audible`; `handle`, `refresh_vocal_lanes`, `vocal_lanes`; `AutomationState::{vocal, lane_for, lane_by_id}` |
| Row layout | `resonance-app/src/state/arrange_layout.rs` | `ArrangeRowKind::AutomationLane { track, lane }` |
| Track header | `resonance-app/src/view/track_header/track.rs`, `track_header/automation_lane.rs` | "N auto" caret, `view_automation_lane_header` |
| Mixer picker / tint | `resonance-app/src/view/mixer/automation.rs`, `view/mixer/inspector/automation.rs` | `choices_for`, `add_lane_picker`, `live_value`, `automation_group` |
| Engine storage | `resonance-audio/src/engine/automation.rs` | `AutomationLanes`, `AutomationSnapshot { mix_lanes, plugin_params }`, `ResolvedParamLane`, `LiveValueEmitter` |
| Engine application | `resonance-audio/src/mixer/automation_apply.rs`, `engine/midi/outbound.rs` | `auto_gain_ramp`, `auto_volume_ramp`, `auto_master_volume`, `auto_muted`, `apply_plugin_params` (every `PARAM_AUTOMATION_CADENCE` = 64 frames), `emit_device_param_automation` |

Invariants:
- The engine owns the authoritative lanes. The app edits its mirror
  optimistically and sends the **whole lane** (`SetAutomationLane`) on every
  edit. The echo is applied idempotently.
- The app allocates lane ids (`alloc_lane_id`) and never reuses one. After a
  load or undo restore the allocator is bumped past every present id.
- The audio thread only reads the published `AutomationSnapshot` (ArcSwap),
  which holds enabled lanes only. Sampling is a binary search with no
  allocation. Plugin ranges are resolved on the engine thread.
- Every lane edit goes through `update()`, so the frozen gate and undo
  classification apply to GUI and control-API edits alike.

Engine application in detail (gain ramps, PDC-compensated evaluation, render
paths) belongs to [mixer-render-core.md](../audio-engine/mixer-render-core.md).

## Surfaces

- **GUI:** Mixer → inspector → AUTOMATION group (add, Read, remove). Arrange →
  track header "N auto" caret; timeline band and lane rows for point edits.
  The Delete key uses the `TimelineDeleteSelection` keymap command.
- **Control API / MCP:** the `automation.*` namespace, see
  [automation-control-api.md](./automation-control-api.md).
  `song.tracks`/`song_tracks` and `master.summary`/`master_summary` list lanes
  without their points.
- **Agent skills:** `resonance-agent-plugin/skills/mixing` and `mastering` (via
  the control API).

## How it's tested

| What | Where | Run |
|---|---|---|
| Model sampling and value maps | `resonance-common/tests/automation.rs` | `./scripts/run-tests.py -p resonance-common` |
| Edit reducers, mirror, live tint, persistence, device lanes | `resonance-app/tests/timeline/automation_{edits,mirror,live_tint,persistence,device_params}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Canvas geometry and input math | `resonance-app/tests/timeline/timeline_automation_{input,render,device_lanes}.rs` | same |
| Lane rows, chip cycle, header alignment | `resonance-app/tests/timeline/timeline_automation_{lane_rows,lane_selector,group}.rs`, `track_header_automation_lane_rows.rs` | same |
| Goldens | `resonance-app/tests/snapshots/timeline_automation_*-wgpu.png`, `track_header_automation_lane_rows_expanded-wgpu.png`, `timeline_vocal_curve_rows_expanded-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test timeline` |
| Vocal curve rows: four rows on expand, unsupported placement ignores a click, golden (live / dimmed / greyed) | `resonance-app/tests/timeline/timeline_vocal_curve_rows.rs` | `./scripts/run-tests.py -p resonance-app` |
| Vocal curve model, sampling, point edits, old configs load, `plan_set_lane` | `resonance-app/tests/vocal/expression_curves_model.rs` | same |
| Vocal curves over the control API, persistence, undo, engine never sent | `resonance-app/tests/control/control_vocal_curves.rs` | same |
| Mixer picker and strip tint | `resonance-app/tests/mixer/mixer_automation_controls.rs` | `./scripts/run-tests.py -p resonance-app` |
| Prune on plugin remove / swap, undo | `resonance-app/tests/mixer/plugin_removal_prunes_lanes.rs` | same |
| Engine command boundary | `resonance-audio/tests/engine/automation_handlers.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Per-block application, block-size independence, PDC | `resonance-audio/tests/mixer/automation_{render,block_size,comp_delay,live_value}.rs` | same (also with `RESONANCE_RENDER_THREADS=8`) |
| Device-param CC/NRPN emission | `resonance-audio/tests/midi_hw/device_param_automation.rs` | same |
| Offline renders play automation | `resonance-audio/tests/bounce/automation_render.rs` | same |

Not covered: the actual pointer gestures on the canvas (drag, double-click,
right-click) run only through geometry helpers, not through a simulated
`TimelineCanvas` event. There is no live check that device-param lanes reach
real hardware.

## Decisions and deviations

- **One lane per target, whole-lane replace.** This keeps engine, mirror and
  file in step with one command. The cost is that every point edit re-sends
  the whole lane.
- **Plugin lanes are linear in plain units.** A perceptual sweep on a skewed
  parameter needs many points. `automation.shape` writes them. The engine has
  no curve kinds beyond linear and stepped.
- **Frames, not ticks.** Points are absolute. Only `CommitBpm` and bar
  insert/remove re-anchor them. Global tempo/meter events deliberately do not,
  which matches clips and markers.
- **Vocal curves are a projection, not stored lanes.** One curve per section
  shown at every placement could not be one engine lane in frames; storing
  it in the lane config keeps it with the section (resize, copy, undo).
- **Lanes are created only in the mixer.** The inspector notes that its
  per-row "show in Arrange" jump from the mixer-cleanup spec is not built.

## Known gaps

- There is no automation recording (write, touch or latch from fader or knob
  moves). See [../../intents/timeline/automation-recording.md](../../intents/timeline/automation-recording.md).
- You cannot add a lane from the Arrange view. Bus and master lanes do not show
  on the Arrange view. You can select only one breakpoint at a time, and there
  is no copy/paste of points. There is no "show in Arrange" jump from the
  mixer. See [../../intents/timeline/arrange-automation-editing.md](../../intents/timeline/arrange-automation-editing.md).
- Removing bars leaves the points inside the removed span in place, which can
  stack them on top of later points.
- Device-param lanes are read-only on the control API.
