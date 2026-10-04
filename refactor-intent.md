# Refactor intent — architecture epics

Written 2026-09-26 at master `64ebb913`; **status updated 2026-10-04**
(code review 2026-10-02, ARCH2-09). Epics A, B, C and E are **done**;
Epic D is down to D-7e/D-7f. The "Current state" blocks below are kept as
the record of where each epic started — they are not today's numbers.

| Epic | Status | Where it landed |
|---|---|---|
| A — one declarative project model | **Done** (A-13j @ 423ed8c1): no `UndoExtras`, one `Reconcile` driver, no catch-all undo arms, `Resonance` at 40 fields. A-14 (delta snapshots) judged not worth it. | `arch-migration-plan.md` → "A-13j landed" |
| B — render-graph publishing | **Done** (B-6): no `RwLock` in `engine/`/`mixer/`, enforced by `engine_and_mixer_take_no_state_lock` | `engine/render_graph.rs` |
| C — engine error taxonomy | **Done** (C-5): `EngineError`, no `Result<_, String>` in audio/common pub fns, enforced by arch-invariants | `types/error.rs` |
| D — app-owned entity ids | **In progress**: D-1…D-5, D-6 (design), D-7a–d landed. **Open:** D-7e (take-group ids in the `GrantIds` grant) and D-7f (delete the engine's `next_clip_id` / `next_take_group_id`, `reserve_clip_id`, the `SetProjectDir` scan) | `docs/design/D-6-engine-created-ids.md`, `arch-migration-plan.md` → "D-7d landed" |
| E — feature-gate `resonance-common`'s model | **Done**: `model`/`decode` features, `plugins_disable_default_features_on_resonance_common` | `resonance-common/Cargo.toml` |

This is the hand-off for the remaining architecture work from the 2026-09-26
review (`code-review-todo.md`, ARCH-01…ARCH-10). Every bug-level finding is
fixed. What is left is structural. It is written so a fresh agent with no
context from that session can pick up one todo and land it.

**Read before starting any todo:** `CLAUDE.md`, `ARCHITECTURE.md`, and the
section of `arch-migration-plan.md` named in the todo. That file holds the
evidence, file/line maps and the measured numbers behind each step. Its
"Progress notes" blocks record where the landed work diverged from the plan.
Line numbers in both files drift, so re-locate by symbol.

---

## 0. What already landed (don't redo)

| Item | Landed | Where to look |
|---|---|---|
| ARCH-01 A1-1 | Undo fixed-point test through both restore paths | `resonance-app/tests/io/undo_snapshot_fixed_point.rs` |
| ARCH-01 A1-2 (1)(2)(9a) | `clip_fade_gain`, `compose_arrangements` removed from `UndoExtras`; `ProjectFile.chord_track` persisted | `undo/snapshot.rs`, `project/model.rs` |
| ARCH-02 A2-1 | Per-map `try_read` miss counters | `SharedState::lock_misses`, `StateMap`, `CycleLoadReport` |
| ARCH-02 A2-2 | Deferred-drop retire queue; all ArcSwap publishers go through it (MIX-04) | `resonance-audio/src/engine/retire.rs` (`retire::publish`) |
| ARCH-02 A2-3 | Quantize/humanize/tuning caches computed off-lock | `engine/midi/clips.rs`, `engine/vocal_render.rs` |
| ARCH-03 | resonance-audio tests 130 → 12 binaries; `test-internals` feature; `test_support` (was `__test_support`) | `resonance-audio/tests/<group>/`, `Cargo.toml` |
| ARCH-04 A4-1..3 | Id-collision invariant test; all app id bases in one table; `Resonance::allocate_track_id` skips group ids; engine hint rule fixed | `resonance-app/src/state/ids.rs`, `tests/io/id_allocation.rs` |
| ARCH-05 A5-1/2 | `tracing` everywhere, no `eprintln!` in library crates, nothing logs on the audio thread | `tools/arch-invariants`, `resonance-plugin/src/logging.rs` |
| ARCH-06 A6-1 (+ H2 slice) | 17 sub-message enums moved beside their handlers; `message.rs` 1771 → 1084 lines | `update/<domain>.rs`, `pub use` in `message.rs` |
| ARCH-07 A7-1/2 | `flush_denormals` → `resonance-dsp`; 8/11 plugins dropped `resonance-common`; allow-list invariant | `tools/arch-invariants` (`PLUGIN_COMMON_ITEMS`, `PLUGINS_ON_COMMON`) |
| ARCH-08 | Plugin manifests platform-neutral; iced UI moved out of the SDK | `resonance-app/src/plugin_ui.rs` |
| ARCH-09 A9-1/2 | History capacity in the app; plugin blobs `Arc<[u8]>`; cheap gesture-change check | `undo/history.rs`, `Resonance::gesture_changed_since` |
| ARCH-10 | Layering/test-layout/logging rules are tests | `cargo test -p arch-invariants` |

---

## 1. Working protocol for agents (learned the hard way)

- **One todo per agent, in its own git worktree and branch** (`arch/<epic>-<step>`).
  Start with `git merge --no-edit master`, then check the base with
  `git merge-base --is-ancestor <expected-master-sha> HEAD`. Worktrees have
  started from a stale base before.
- **Memory:** `export CARGO_BUILD_JOBS=3`, and **no more than about 4 concurrent
  builds machine-wide**. Six or seven parallel app builds OOM-killed this
  machine. Iterate with targeted tests
  (`cargo test -p resonance-app --test <group> <filter>`) and run
  `./scripts/run-tests.py -j4 -p <crates> -p arch-invariants` once at the end.
- **Test rules:** these are enforced by `arch-invariants`.
  - Add modules to existing group binaries; never add a new top-level test file
    in `resonance-app/tests/` or `resonance-audio/tests/`.
  - No inline `#[cfg(test)]`.
  - Build the app with `Resonance::new_for_test*()`.
  - Re-bless goldens with `RESONANCE_BLESS=1`, debug profile, after checking them
    visually. This machine is canonical.
  - Silent goldens prove nothing: assert non-silence per scenario.
- Never run crate-wide `cargo fmt`; hand-format changed lines.
- **A new crate** needs a row in `tools/arch-invariants` `allowed_internal_deps`.
- **Test first.** Every step has a guard test that fails before the change or
  pins behaviour, landed in the same branch.
- **Behaviour:** these are refactors. Any audible or user-visible change must be
  called out in the commit and the hand-back.
- **Orchestrator:** merge each branch into master yourself (`--no-ff`), re-run the
  affected suites, and tick the item in `code-review-todo.md` (the ARCH-xx
  heading plus the status table).
- **Commit trailer:** `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`

---

## 2. Epics

The epics are ordered by value ÷ risk. Section 3 shows which can run in parallel.

### Epic A — "State tax": one declarative project model  (ARCH-01, ARCH-06, ARCH-09) — DONE

**Why.** Adding one persisted, undoable field means touching 8+ files:
`Resonance`, `ProjectFile`, `serialize.rs`, two replay paths (`replay/` slow,
`replay_diff.rs` fast), `UndoExtras`, `engine_events`, and the control
`view_model`. The two restore paths have drifted repeatedly; the review and
fix campaign found about ten asymmetries between them. `message.rs` and `lib.rs`
are the merge-conflict hubs for parallel agents.

**Current state (2026-09-26 — historical; see the status table).**
- `UndoExtras` still has 8 fields: `compose_derived_clips`,
  `compose_next_derived_clip_id`, `vocal_clip_lyrics`, `automation_lanes`,
  `reference`, `track_freeze`, `external_instruments`,
  `external_instrument_devices`.
- `Resonance` has 88 fields.
- `message.rs` has 15 enums left: Arrangement, Track, Bounce, Mixer, Clip,
  Plugin, Take, Viewport, ProjectIo, RecoveryChoice, Ui, Import, Pool, Relink,
  plus `Message`.
- `undo/classify.rs` has 32 catch-all arms (`(_) => Skip` ×25, `(_) => Record` ×7).
- `ProjectFile` has no `PartialEq`, so `same_state` compares via `serde_json`.
- `MidiClipState.notes` is `Vec<MidiNote>`, deep-copied per snapshot.

**Done when.**
- `UndoExtras`, `pending_undo_extras` and `finalize_undo_restore` are deleted:
  an undo snapshot is just `LoadedProject` (`ProjectFile` + notes + blobs).
- Both restore paths go through one per-domain `Reconcile` trait.
- `classify.rs` has no catch-all arms.
- `Resonance` has ≤ 40 fields.
- The fixed-point test stays green throughout.

**Todos** (plan: `arch-migration-plan.md` → "ARCH-01", "ARCH-06", "ARCH-09"; landing order below):

1. **A-1 `external_instruments` + `_devices` from `ProjectFile`** (A1-2 step 3).
   - Change `restore_external_instruments(&UndoExtras)` to read
     `ProjectTrack.external_instrument` (`device_id` lives there).
   - Remove the slow path's double restore (`replay/entity.rs` vs
     `finalize_undo_restore`).
   - Guard: fixed-point test. Size: ~−40/+20.
2. **A-2 `vocal_clip_lyrics` from `ProjectFile`** (A1-2 step 4).
   - `apply_compose` reads `midi_clips[*].vocal_lyrics`.
   - The slow path pads lyrics to the note count and the snapshot doesn't.
     Normalise first, then compare (see H2 notes and FU-H2c in
     `code-review-todo.md`).
   - Size: ~−15/+10.
3. **A-3 `automation_lanes` from `ProjectFile`** (A1-2 step 5). `replay_diff.rs`
   builds the lane map from `automation_lanes` the way `replay/mod.rs` already
   does. Size: ~−10/+10.
4. **A-4 `track_freeze` from `ProjectTrack.freeze`** (A1-2 step 6).
   - Keep `apply_freeze_restore`'s cache-deletion semantics.
   - Keep U1's rule that `freeze.content_baselines` survive a full-replay undo
     (FU-H2b).
   - Delete the `freeze_rehydrate` special case in `engine_events/project_io.rs`.
5. **A-5 `reference` split** (A1-2 step 7).
   - First split `ReferenceUndo` into content (file, trim, gain) and monitor
     state (`ab_source`, `loop_to_mix`, meters).
   - Persist content in `ProjectFile`. Monitor state is not undoable.
6. **A-6 `compose_derived_clips` + counter** (A1-2 step 8).
   - Note the direction changed: since U1 (FU-H2a) both paths use
     `Resonance::restore_derived_clips` *from the snapshot map*, because
     rebuilding drops entries whose `MidiClipCreated` echo is still in flight.
   - To delete this extra, the derived-clip mapping `(section, placement,
     track) → ClipId` must become part of `ProjectFile` (e.g. on
     `ProjectMidiClip` as `derived_from`), plus a monotonic counter.
   - Design first; this is the riskiest of the folds.
7. **A-7 delete `UndoExtras`** (A1-2 step 9b).
   - Once A-1…A-6 are in, delete `UndoExtras`, `pending_undo_extras` and
     `finalize_undo_restore`.
   - Replace the `is_none()` checks with an explicit `io.restoring_undo: bool`.
8. **A-8 `PartialEq` on the `ProjectFile` tree** (cheap half of A9-3).
   - About 20 one-line derives across `project/{model,sections}.rs`,
     `compose/{drumroll/*,generate,lane_generator}.rs`, `state/markers.rs`,
     `resonance-audio/src/types/tempo/map.rs` and
     `resonance-music-theory/src/generator/mod.rs` (`GeneratorSpec`).
   - Then `same_state` / `gesture_changed_since` compare structs instead of JSON.
   - The probe `tests/timeline/undo_history.rs::snapshot_cost_probe_on_demo_project`
     shows the JSON compare is most of the 692 µs gesture check.
9. **A-9 notes as `Arc<Vec<MidiNote>>`** (A9-3).
   - `MidiClipState.notes` and `LoadedProject.midi_notes`; ~23 mutation sites use
     `Arc::make_mut`, reads compile through `Deref`.
   - Guard: snapshot a 2 000-note project 200× after editing one clip; every
     other clip's notes are `ptr_eq` across snapshots.
   - Conflict: medium, `engine_events/midi.rs` and the compose/vocal paths.
10. **A-10 exhaustive `undo_action` per enum** (A6-4).
    - `impl XMessage { fn undo_action(&self) -> UndoAction }` beside each enum,
      with no `_` arm.
    - `classify.rs` becomes `Message::X(m) => m.undo_action()`.
    - Start with the 25 `Skip` catch-alls, then the 7 `Record` ones (this also
      removes transient-variant double snapshots).
    - Done-when: `grep -nE '\(_\) => UndoAction::(Skip|Record)' undo/classify.rs`
      is empty. Add that grep as an `arch-invariants` test.
11. **A-11 move the remaining enums** (A1-3 remainder).
    - Move Arrangement, Track, Bounce, Mixer, Clip, Take, Viewport, ProjectIo,
      Import, Pool and Relink beside their handlers as a pure move with
      `pub use` re-exports.
    - Leave `Ui` and `Plugin` for last; they have the highest churn.
    - Do these before or alongside A-10.
12. **A-12 `Resonance` sub-states** (A6-2/A6-3). One todo per sub-state, each a
    mechanical rename:
    - `PluginCatalog` (available / scan_failures / scan_in_progress)
    - `MidiDevices`
    - `Banners`
    - `InputDevices`
    - `PresetState`
    - `ModalState`
    - `PluginMirror`
    - `MasterState` (touches `undo/snapshot.rs`, `serialize.rs`, `view_model`; do it last)
13. **A-13 `Reconcile` trait** (ARCH-01 step 3, the capstone).
    - One per-domain `trait Reconcile { fn reconcile(r, old: &ProjectFile, new: &ProjectFile) }`
      drives both `replay_loaded_project` (old = empty) and `try_diff_replay`
      (old = current).
    - Delete the per-domain branches in both paths.
    - Carry the known asymmetries across: `restore_performance` now runs on both
      paths (U1), and so does the derived-clip restore.
    - Do this only after A-7, one domain per todo.
14. **A-14 delta snapshots** (A9-4). Only after A-13; one diff engine, not two.
    Optional.

**Conflicts.**
- `undo/**`, `update/project_io/replay*/**` and `engine_events/project_io.rs` are
  shared by A-1…A-7 and A-13. Run those strictly one at a time.
- A-8, A-11 and A-12 touch different files and can run beside them.
- ba #1059 (a July `message/` directory split, 124 files diverged) is
  unsalvageable; close it or re-scope it to A-11.

### Epic B — Engine render-graph publishing  (ARCH-02 remainder) — DONE

**Why.** The audio callback `try_read`s five
`Arc<parking_lot::RwLock<…>>` maps: `tracks`, `busses`, `master`, `clips`,
`midi_clips` (`resonance-audio/src/engine/mod.rs`). Control-thread `write()`s
make it drop a block whenever a writer is queued, because parking_lot is
task-fair. The target is an engine-thread-owned `EngineModel` that publishes an
immutable `RenderGraph` via `ArcSwap`, so the callback does one `load()` and
never fails. The diagram is in `arch-migration-plan.md` → "ARCH-02 / Target
model".

**Gate — do this first (B-0).** Since MIX-02's offline-render gate, offline
paths no longer cause callback dropouts. Run a real editing session (note drags,
quantize, control-API bulk writes, project load) and read
`SharedState::lock_misses` from the cycle-load report (stderr at info level).
- **If the per-map counters stay at 0,** this epic is engine-thread latency and
  determinism hygiene. Schedule it after epics A, C and D.
- **If they don't,** start with the map that misses.

**Done when.** No `RwLock` in `HandlerCtx` or `ChunkCtx`, no `try_read` in
`mixer/`, and `grep '\.write()' resonance-audio/src/engine` returns only the
plugin mutex. The A2-9 hammer test (500-clip project, continuous render, heavy
edits) shows zero contended blocks and zero retire-queue drops on the render
thread.

**Todos** (plan: `arch-migration-plan.md` → "ARCH-02 / Migration map" + "Steps" A2-4…A2-9; each ends with `grep '\.write()'` empty for its map):

1. **B-1 `midi_clips` → `RenderGraph`** (A2-4). Start the `RenderGraph` struct
   with this one field and publish through `retire::publish`.
   - Change the 22 write sites in `engine/midi/{clips,live}.rs`.
   - Drop one `try_read` in `mixer/callback/play.rs`.
   - Update `BlockInputs.midi_clips: &[Arc<MidiClip>]`, bounce/freeze/stem
     readers, and both harnesses.
   - Update the audio tests that hand-build `Arc<RwLock<Vec<MidiClip>>>`; route
     them through `EngineHandlerHarness` while there.
   - Size: ~400 lines across ~35 files. Conflict: medium (any engine MIDI
     handler change).
2. **B-2 `busses` + `master`** (A2-5).
3. **B-3 `tracks`** (A2-6). Route the one worker writer
   (`bounce_realtime.rs`, which removes the target track on cancel) through the
   engine thread.
4. **B-4 `plugins`** (A2-7). `Arc<PluginSlot>` shares the instance mutex. The
   retire queue must drop old graphs on the engine thread only once
   `strong_count == 1`, so `ClapInstance::drop` never runs on the audio thread.
   Keep `try_lock_with_backoff`.
5. **B-5 `clips`** (A2-8). This is the most cross-thread; do it last.
   - First change `ClipSource::Memory(Vec<f32>)` to `Arc<[f32]>`.
   - Add an internal engine-thread message (`EngineInternal::ClipLoaded`,
     `TuningCachesBuilt`, `PitchAnalysed`) for the load, freeze and analysis
     workers.
   - Rewrite `take_park.rs`'s "mutate while holding `clips.write()`" contract as
     plain sequencing.
   - Keep V6's `PersistClipWavs` semantics (clip WAVs are persisted before
     undo-visible edits).
6. **B-6 hammer test + delete the locks** (A2-9).

**Conflicts.** The engine handlers are hot, so run this epic's todos
sequentially. They don't touch `resonance-app` beyond harness accessors.

### Epic C — Engine error taxonomy  (ARCH-05 remainder) — DONE

**Why.** `AudioEvent::Error(String)` has 41 emit sites, and the app can only show
a banner for them. There are 48 `Result<_, String>` in resonance-audio and 20
in resonance-common. Nine typed precedents already exist in `AudioEvent`
(`ExportError { kind }`, `PluginLoadFailed { reason }`, …). The review's claim
that "the control layer string-matches" was wrong: control errors are decided
against the app mirror. The value here is consistency, plus typed `kind`s for
failed control jobs.

**Done when.**
- No `AudioEvent::Error(String)` remains.
- `BounceError`, `TrackBounceError` and `StemExportError` carry typed errors.
- `JobStatus.error` has a `kind`.
- resonance-audio/common public fns don't return `Result<_, String>`
  (`resonance-amp`'s NAM loader is exempt).

**Todos** (plan: `arch-migration-plan.md` → "ARCH-05" A5-3/A5-4):

1. **C-1 `EngineError { kind, message }`**.
   - New `resonance-audio/src/types/error.rs` with
     `EngineErrorKind { NotFound, Busy, Unsupported, Io, Plugin, Internal }`,
     mirroring `resonance_control::ErrorKind` plus Io and Plugin.
   - Two passes: first `EngineError::internal(msg)` everywhere, a one-token
     change per site; then classify (`clips.rs` is mostly NotFound,
     `plugins.rs` mostly Plugin).
   - `AudioEvent::Error(EngineError)`. The app's banner keeps `.message`
     (`engine_events/dispatch.rs` → `transport::error`).
2. **C-2 typed bounce/export errors**. The three bounce error variants carry
   `EngineError`, or fold into `ExportErrorKind`. Add an optional `kind` to
   `JobStatus.error` (resonance-control; additive, no `PROTOCOL_VERSION` bump)
   and update `update/control/job.rs` and the MCP job tool description.
3. **C-3 `thiserror` per module in resonance-audio**. One file per commit
   (`midi_io`, `midi_hardware`, `recording`, `io/wav`, `types/clip`,
   `clap_host/bundle`, …). Each error type lives beside its module and converts
   into `EngineError` at the event boundary.
4. **C-4 `thiserror` in resonance-common** (20 sites).
5. **C-5 invariant**: `arch-invariants` forbids new `Result<_, String>` in pub fns
   of audio/common (allow-list the NAM loader).

**Conflicts.** `engine/clips.rs` and `plugins.rs` are shared with epic B. Run C-1
and C-2 either before B starts or between B todos, not concurrently.

### Epic D — App-owned entity ids  (ARCH-04 remainder) — IN PROGRESS (D-7e, D-7f open)

**Why.** Ids are allocated in two places:
- The engine has counters `next_clip_id` / `track` / `plugin` / `group` /
  `take_group` / `send` / `bus` / `ref` / `asset` / `marker`, with about 15
  `max(id+1)` bump sites and two reserve commands (`ReserveAssetIds`,
  `RestoreTakeGroups`).
- The app has the bases in `state/ids.rs`.

The ranges are partitioned by hand. A4-1's invariant test already caught one
real collision (fixed). Each new entity type re-decides who owns its ids.

**Done when.** `grep -n 'next_[a-z_]*_id' resonance-audio/src/engine` is empty.
The app allocates every id; the engine rejects an add without an id, or with a
colliding one, via an error event. `ReserveAssetIds` and the take-group bump
are gone. `tests/io/id_allocation.rs` stays green and gains a case per space.

**Todos** (plan: `arch-migration-plan.md` → "ARCH-04" A4-4; order by how mechanical each is):

1. **D-1 plugins**: flip the `id_hint: None` plugin adds to
   `allocate_plugin_id`; the engine rejects a missing or duplicate id; delete
   `next_plugin_id`, the hint-vs-base rule and `CONTROL_PLUGIN_ID_BASE`.
2. **D-2 sends**. **D-3 busses** (fold in the return-bus base).
   **D-4 tracks** (plus `demo.rs` and templates; fold in the sub-track base).
   **D-5 references / markers.**
3. **D-6 design todo, write the design before code.** Recording clip ids,
   take-group ids and import asset ids are created engine-side at times the app
   can't pre-decide. Proposal: the app reserves a block at arm/import time (e.g.
   `AudioCommand::ArmTrack { clip_ids: Range<u64> }`), the engine never invents
   one, and running out mid-take is a defined error. Must respect STATE-08
   (clip ids monotonic across `ClearAll`/replay), V6's `PersistClipWavs`, and
   M12/A4's import epochs. Then implement it as D-7.

**Conflicts.** The engine handlers overlap with epic B's maps. Interleave per
entity: after B-3 (tracks), D-4 is easier because the engine thread owns the
model.

### Single todo E — Feature-gate the model inside `resonance-common`  (ARCH-07 A7-3) — DONE

**Why.** Plugins can still name DAW model types through `resonance-common`,
reached via the SDK. A7-1's allow-list is a source-level guard; this makes it a
type-level one, and makes a standalone `cargo build -p <plugin>` lean.

**Todo.**
- Add `[features] default = ["model", "decode"]` to `resonance-common`:
  - `model` gates the nine model modules (take, midi_map, device_definition,
    automation, freeze, device_registry, group_identity, external_instrument,
    track_group), plus optional `dirs` and `time`.
  - `decode` gates `wav` and `audio_probe`, plus optional `symphonia`.
  - `serde_json` stays unconditional.
- `resonance-plugin` and the three plugins that still depend on common use
  `default-features = false`; drums and ir add `features = ["decode"]`.
- Extend `arch-invariants`: any plugin's `resonance-common` dependency must set
  `default-features = false`.
- Size: ~60 lines. Conflict: low.
- Do **not** split out a `resonance-model` crate (A7-4); that's 103 importing
  files for no extra guarantee.

---

## 3. Parallelism map

| Can run together | Must be sequential |
|---|---|
| Epic A (one todo at a time in `undo/`/`replay*`) ‖ Epic E ‖ A-8 / A-11 / A-12 | A-1 → … → A-7 → A-13 → A-14 |
| Epic C ‖ Epic A | B-1 → … → B-6 |
| Epic D (D-1…D-5) ‖ Epic A | C-1/C-2 not concurrently with B todos (shared `engine/clips.rs`, `plugins.rs`) |
| | D-6 design before D-7; D-4 easier after B-3 |

Suggested first wave (≤ 4 agents): **A-1**, **A-11** (pure moves), **E**, **C-1**.
Then B-0: read the counters from a real session and decide epic B's priority.

---

## 4. Not in these epics (open, need a human)

- **macOS** (FU-M1a, M8a, M8c, H4a): the Cocoa changes from PLG-01/03/05 and the
  async destroy are only type-checked. Run `cargo check` and the three ignored
  Cocoa tests on a Mac (commands are in CLAUDE.md).
- **FU-B1** (product decision): a section resize re-rolls generated chord and
  vocal lanes, which loses hand edits.
- **LIB-06** (confirm the sound change): compressor release now matches the knob.
  Revert `cea54427` if unwanted.
- All other open follow-ups are listed at the top of `code-review-todo.md`.
