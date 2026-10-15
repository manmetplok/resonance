# Undo and redo

> Verified against master @ `acac5ea3` on 2026-10-15.
> Spec history: `docs/history/refactor/refactor-intent.md` (Epic A),
> `docs/history/refactor/arch-migration-plan.md` (ARCH-01, ARCH-06 A6-4,
> ARCH-09), `docs/history/refactor/A-13-reconcile.md`;
> `docs/history/reviews/code-review-todo.md` (STATE-02, STATE-07, STATE-10,
> CTL-03, VIEW-18, FU-A2a, FU-V5b).

Every edit to the song — from the GUI or from the control API — can be taken
back and re-applied. One user action (a click, a drag, a fader sweep, a
recording pass, one control call) is one undo entry, and the history is shared
between the user and any connected agent.

## Behaviour

### What counts as one entry

Each `Message` is classified before it is dispatched (`undo::classify`, which
delegates to an exhaustive `XMessage::undo_action()` beside each handler):

- **Skip** — UI-only state, selection, view switches, engine echoes, dialog
  state, async completions. Never an entry, never dirty, never clears redo.
- **Record** — an atomic edit: snapshot the pre-edit state, push one entry.
- **RecordCoalesced(key)** — a continuous control (track/bus/master volume
  and pan, plugin param, send level, marker drag, preset ◀/▶ steps, text
  typed in a name or lyric field, drum-group and lane knobs). Consecutive
  messages with the same `CoalesceKey` merge into the run's first entry; any
  other action breaks the run.
- **Begin / Commit** — a gesture (clip move/trim/fade/gain, MIDI clip
  move/trim, loop drag, tempo-event drag, automation breakpoint drag). Begin
  snapshots; Commit records **only if the project actually changed**, so a
  click without a move is no edit and keeps the redo stack.
- **Recordings** land through `record_recording_edit`: all clips, takes and
  live-MIDI captures of one recording run coalesce into a single "Record"
  entry. A take that lands mid-drag splits the drag around it.
- **The vocal-melody question** (BUG-70): a GUI chord edit that would
  rewrite a vocal melody records its entry as usual; the question's Apply
  (classified `Skip`) mutates and calls `mark_edited_without_history`, so
  it rides that entry, and Cancel undoes it and drops the redo step
  (`UndoHistory::drop_redo_top`). The question holds the history's
  `generation` (bumped by every stack change) and closes if it moves.
- **Control calls**: `control::execute` wraps every mutating call in a
  compound group (`with_compound_undo`), so a call that fans out into many
  messages is one entry and one revision bump. A failure part-way leaves the
  partial edit as one undoable entry.

A Record or a committed gesture marks the project dirty and bumps the
`revision` counter that every mutating control reply carries.

### What a snapshot is

`UndoSnapshot` wraps a `LoadedProject` — the same `ProjectFile` + MIDI notes +
plugin blobs a disk load uses — built by `snapshot_for_undo`. MIDI notes and
plugin blobs are `Arc`-shared, so an unchanged clip costs a refcount. Taking
a snapshot also sends `PersistClipWavs` so every clip it names has a WAV. A
preset load reserves a "late" blob slot so undoing it restores the plugin's
full prior state, not a stale cached blob. So does a snapshot taken while a
plugin's own state change is waiting for its fresh blob (an editor kit pick,
STATE2-07): the change sends a `SavePluginState` with a request id
(`plugin_mirror::OwedBlob`), and only the `PluginStateSaved` echoing that id
fills the slot — never an earlier refresh still in flight, such as a plugin
add's seed or an editor close (BUG-39). An undo / redo marks the refreshes in
flight superseded: their echoes fill their slots but leave the cache, which
holds the restored blob. A change made after the restore owes a live echo
again. The reference A/B monitor state is stripped (not undo state).

### Undo / redo

- `Ctrl+Z`, `Ctrl+Shift+Z` (and `Ctrl+Y`) in the GUI; `edit.undo` /
  `edit.redo` over the control API.
- Restores in place through the [reconcile driver](reconcile.md), diffing the
  target against the live project: no `ClearAll`, plugins that are kept stay
  alive, only what changed is sent to the engine. Playback stops.
- Refused while a recording runs, a gesture is open, a save is collecting, an
  offline bounce runs or a freeze renders (`can_undo_redo_now`). An empty
  stack is a no-op, not an error.
- A banner says "Undid <label>" / "Redid <label>". Labels come from
  `undo::describe` ("track volume", "add plugin", …).
- Capacity: 200 entries (`DEFAULT_HISTORY_CAPACITY`), oldest dropped.
- The history is cleared by open, new and template instantiation. It works on
  an untitled project (its clip WAVs anchor to the session scratch dir), but
  not while no project is active or while a load replays.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Record dispatch | `resonance-app/src/undo/mod.rs` | `record_undo`, `commit_undo_gesture`, `record_recording_edit`, `with_compound_undo`, `continue_as_one_undo`, `mark_edited_without_history` |
| Classification | `resonance-app/src/undo/classify.rs` + each `update/<domain>.rs` | `UndoAction`, `classify`, `XMessage::undo_action` |
| History stack | `resonance-app/src/undo/history.rs` | `UndoHistory`, `CompoundPhase`, `DEFAULT_HISTORY_CAPACITY` |
| Snapshot + restore | `resonance-app/src/undo/snapshot.rs` | `UndoSnapshot`, `CoalesceKey`, `snapshot_for_undo`, `gesture_changed_since`, `can_record_undo`, `can_undo_redo_now`, `try_undo`, `try_redo`, `restore_from_snapshot_against` |
| Labels | `resonance-app/src/undo/describe.rs` | `describe` |
| Hook in the loop | `resonance-app/src/update.rs` | `update_inner` (classify → dispatch → commit) |
| State | `resonance-app/src/state/session.rs` | `SessionMetaState { dirty, revision, session_id, undo }` |
| Control | `resonance-app/src/update/control/edit.rs`, `update/control/mod.rs` | `status`, `undo`, `redo`, `execute` |

Invariants:
- Everything undoable must be in `ProjectFile` (+ notes + blobs). A gesture
  that edits persisted state outside it leaves no entry
  (`tests/timeline/undo_noop_gesture.rs` guards this).
- No catch-all arm in any `undo_action` (enforced by
  `arch-invariants::undo_classification_has_no_catch_all_arms`).
- `build_project_file(restore(snapshot)) == snapshot.file` (fixed point).

## Surfaces

- **GUI:** Edit commands Undo / Redo (`Ctrl+Z`, `Ctrl+Shift+Z`, `Ctrl+Y`),
  command palette; history banner.
- **Control API / MCP:** `edit.undo` → `mcp__resonance__edit_undo`,
  `edit.redo` → `mcp__resonance__edit_redo`, `edit.status` →
  `mcp__resonance__edit_status` (top labels, `can_undo`/`can_redo`). Every
  mutating reply carries `revision`; a jump bigger than the client's own
  calls means the user edited concurrently.
- **Agent skills:** `resonance-agent-plugin/skills/mixing` checks
  `edit_status` before starting; `mixing` and `mastering` use `edit_undo` to
  back a move out.

## How it's tested

| What | Where | Run |
|---|---|---|
| Stack, capacity, coalesce, snapshot cost probes | `resonance-app/tests/timeline/undo_history.rs` | `./scripts/run-tests.py -p resonance-app` |
| Coalescing through `update` | `resonance-app/tests/timeline/undo_coalesce.rs` | same |
| No-op gestures | `resonance-app/tests/timeline/undo_noop_gesture.rs` | same |
| Recording is one entry | `resonance-app/tests/timeline/recording_undo.rs` | same |
| Undo before an engine echo | `resonance-app/tests/timeline/undo_before_echo.rs` | same |
| View/dialog state is no edit | `tests/timeline/undo_view_state_keeps_redo.rs`, `undo_transient_dialog_state.rs` | same |
| Fixed point, diff shape | `resonance-app/tests/io/undo_snapshot_fixed_point.rs`, `undo_diff_shape.rs`, `undo_restore_flag.rs` | same |
| Control contract | `resonance-app/tests/control/control_edit_undo.rs`, `control_undo_contract.rs` | same |
| Per-domain undo | e.g. `tests/plugins/plugin_param_undo.rs`, `tests/performance/reference_undo.rs`, `tests/mixer/delete_track_confirm_undo.rs` | same |
| Owed plugin blobs (kit pick echoes, by request id) | `resonance-app/tests/plugins/plugin_live_values.rs` | same |
| Catch-all guard | `tools/arch-invariants/tests/architecture.rs` | `cargo test -p arch-invariants` |

Not covered: undo against a real engine (all app tests use the
command-capturing engine), and very long sessions near the 200-entry cap with
large plugin blobs.

## Decisions and deviations

- Whole-project snapshots, not deltas. A-14 (delta snapshots) was judged not
  worth it: a snapshot costs about 0.28 ms on the demo project.
- Since A-13j there is one restore path; the old `ClearAll` full-reload
  fallback and the `UndoExtras` side-car are gone.
- The stack is shared with control clients on purpose; `edit.status` exists
  so an agent can see what it would undo before doing it.

## Known gaps

- History capacity is a constant, not a setting.
- No history panel; only the top label of each stack is visible.
