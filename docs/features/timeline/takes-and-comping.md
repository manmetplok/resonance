# Takes and comping (app side)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none in `docs/history/specs/`; epic #15 / design doc #165 and todos #409–#414, #1392–#1401, ba doc #292, cited in code comments.

When cycle-record mode is on, every loop pass recorded on an armed track
becomes its own **take**, stacked in a take lane under the track (one lane
per track + loop slot). The user can solo a take for the whole slot, cut
the comp at the playhead, drag across a take to promote that stretch into
the comp, or delete a take. This doc covers the app side: the mirror, the
editing rules and the lane UI. Capture, the comp render and recording
latency are engine work — see [input-capture.md](../audio-engine/input-capture.md) and
[recording.md](../hardware-io/recording.md).

## Behaviour

**Loop-record mode.** `transport.loop_record_mode` (default off) decides
whether a cycle-recorded run rolls each pass into its own take (`true`) or
records one ordinary clip with no take group (`false`). It is project
state, saved in `ProjectFile::loop_record_mode` and undoable. GUI toggle:
Settings → "Distinct takes per loop pass". With the mode off the single
clip is **not** a merge of the passes: `finalize_recording` trims it
non-destructively to `[loop_in, loop_out]` measured from the record start,
so only the first pass is heard and later passes sit unused in the WAV
past the trim end (see [recording.md](../hardware-io/recording.md#loop-and-cycle-record)).
The Settings hint ("merged into one clip for the whole loop") overstates
this.

**The mirror.** The app never reads take groups from the engine. It rebuilds
them from engine events (`engine_events/takes.rs`):
- `TakeCaptured` appends a take, creating the group on the first pass. The
  group id is stable per **track + loop slot**, so a second run over the same
  loop folds into the same lane. `pass_index` is the take's ordinal in the lane
  (`T1`, `T2`, …).
- `TakeCompChanged` / `ActiveTakeChanged` are confirmations; the mirror adopts
  them verbatim.
- `TakeGroupRemoved` drops a lane.

**What plays.** One rule, shared with the mixer:
`resonance_common::effective_cover` — active take, else comp segment, else
the latest take.

**Editing** (`TakeMessage`, each one undo entry):
- `SetActiveTake` — solo one take across the slot, or clear the solo with
  `None`. Refused for a take the group does not hold (the engine drops such a
  command with no echo).
- `SplitCompAtPlayhead` — cut the comp in two. Refused unless the playhead is
  strictly inside the slot and inside a segment.
- `PromoteTakeSegment` — promote a take over a range. The range is clamped to
  the slot and to the take's own recorded extent (`Take::audible_extent`, read
  from the take, never from a clip lookup). Refused if nothing survives.
  Editing the comp also clears an active solo.
- `DeleteTake` — removes one take via `AudioCommand::RemoveTake`, or the whole
  lane via `RemoveTakeGroup` when it was the last take. Deletion is an engine
  command rather than a comp edit because a take's recording is an ordinary
  clip that only stays silent while the comp "governs" it.
- A refused edit is caught **before** dispatch
  (`Resonance::take_edit_is_refused` runs the same `plan` function), so it
  spends no undo entry and no revision.
- Comp segments are always derived from `TakeGroup::effective_comp` and edited
  through the `resonance_common` comp helpers (sorted, non-overlapping, inside
  the slot). The engine does not validate `SetTakeComp`.

**Lane UI.**
- The track lane always shows a folded **comp ribbon**. A caret on the track
  header (`UiMessage::ToggleTakeLane`) expands the stack of take cards; lanes
  start folded and the expanded set is transient view state.
- Gestures: click a take card → solo (click again to release); drag across a
  card → promote the raw drag range; click the comp ribbon → split at the
  playhead; right-click a card → delete.
- Header cells (`view/track_header/take_lane.rs`) show `Take N` and a chip:
  `SOLO`, `N seg`, or nothing (dimmed) when unused.
- Audio takes draw waveform peaks from a cache keyed by `(group, take)` and
  checked against the recording's `clip_ref`; MIDI takes draw their notes.
- A take whose WAV was missing at load is flagged (`missing_takes`) and kept in
  the comp, like a missing pool asset.
- Soloing a MIDI take in a group that also holds audio takes silences the audio
  path; the lane shows a hint (`active_take_silences_audio`).

**Persistence and undo.** Take groups are saved in `ProjectFile::take_groups`.
Undo/redo rebuilds the mirror from the snapshot (`clear_for_snapshot` keeps the
peak cache) and re-sends the whole store with `RestoreTakeGroups`, which also
re-parks or un-parks deleted takes' recordings.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Shared model | `resonance-common/src/take.rs` | `TakeGroup`, `Take`, `TakeContent`, `CompSegment`, `effective_cover`, `TakeGroup::{effective_comp, remove_take}`, `SlotCover` |
| App mirror | `resonance-app/src/state/takes.rs` | `TakeGroupState` (`take_captured`, `comp_changed`, `active_take_changed`, `remove_take`, `remove_group`, `peaks`, `missing_takes`) |
| Edits | `resonance-app/src/update/takes.rs` | `TakeMessage`, `TakeEdit`, `plan`, `plan_promote`, `plan_delete`, `finish_comp_edit` |
| Engine-event mirror | `resonance-app/src/engine_events/takes.rs` | `TakeCaptured`, `TakeCompChanged`, `ActiveTakeChanged` handlers |
| Lane canvas | `resonance-app/src/view/timeline/takes/{geometry,draw,input,overlay}.rs` | `effective_cover`, `comp_ribbon_band`, `take_card_band`, `silent_ranges` |
| Header cells | `resonance-app/src/view/track_header/take_lane.rs` | `TakeRole` |
| Loop-record mode | `resonance-app/src/update/transport.rs`, `view/settings.rs` | `TransportMessage::SetLoopRecordMode` |
| Persistence | `resonance-app/src/project/model.rs` | `ProjectFile::take_groups`, `loop_record_mode` |

Invariants: the mirror moves only through `comp_changed` /
`active_take_changed` plus the matching engine command; the app carries no
second definition of "what the slot plays" or "what a removal does".

## Surfaces

- **GUI:** Arrange track lanes (comp ribbon, expanded take stack, take-row
  headers); Settings → "Distinct takes per loop pass".
- **Control API / MCP:** only the mode:
  `transport.set_loop_record_mode` (`distinct_takes`) →
  `mcp__resonance__transport_set_loop_record_mode`; every `transport.*` reply
  echoes `loop_record_mode`. There are no methods to list takes, solo, comp or
  delete them. See [intent: takes control API](../../intents/timeline/takes-control-api.md).
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Comp model and cover rule | `resonance-common/tests/take.rs` (50 tests) | `./scripts/run-tests.py -p resonance-common` |
| Mirror from engine events | `resonance-app/tests/timeline/take_group_mirror.rs` | `./scripts/run-tests.py -p resonance-app` |
| Comp edits, refusals, undo | `resonance-app/tests/timeline/take_comp_edits.rs` | same |
| Lane hit-testing and gestures | `resonance-app/tests/timeline/take_lane_input.rs` | same |
| Lane render + goldens | `resonance-app/tests/timeline/take_lane_render.rs`; `tests/snapshots/take_lane_*-wgpu.png` (14 images) | `RESONANCE_BLESS=1 cargo test -p resonance-app --test timeline` |
| Deleted take stays silent | `resonance-app/tests/timeline/take_removal_audible.rs` | `./scripts/run-tests.py -p resonance-app` |
| Persistence + missing media | `resonance-app/tests/io/take_lanes_persistence.rs` | same |
| Engine capture / comp render / removal | `resonance-audio/tests/engine/loop_record_takes.rs`, `tests/mixer/take_comp_render.rs`, `tests/engine/take_removal.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Loop-record mode over the wire | `resonance-app/tests/control/control_transport.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: a live cycle-record session on real hardware; comping across a
tempo change or a bar shift.

## Decisions and deviations

- One lane per track + loop slot across record runs (todo #1392), like Logic /
  Pro Tools / Reaper folders.
- Deleting the last take removes the lane instead of being refused: an empty
  group would fall back to playing the pass just deleted.
- The promote range is clamped to what the take recorded, so a comp never
  contains a silent hole the user did not ask for.
- The `SetLoopRecordModeParams` doc in `resonance-control` says to "comp or
  solo them afterwards with the take methods"; no such methods exist.

## Known gaps

- No control API for takes — [takes-control-api](../../intents/timeline/takes-control-api.md).
- Take groups are sample-anchored and do not move with tempo changes or
  insert / remove bars (see [arrangement-bars.md](./arrangement-bars.md)).
- Recording latency (PDC latch, pass cuts at the loop seam) is engine work;
  code review RT-01 / RT-02 are fixed in `resonance-audio/src/recording.rs`
  (`record_pdc_samples`, `CycleCut`). See [recording.md](../hardware-io/recording.md).
