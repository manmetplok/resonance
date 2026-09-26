# A-13: the `Reconcile` trait, one domain order for both restore paths

Design for `refactor-intent.md` Epic A item 13 (`arch-migration-plan.md`
ARCH-01 step 3, "A-13 roadmap" in the A-7 progress note). Written against
master `d6d89413`. This document covers the trait, the driver, and the first
slice (A-13a, roadmap group 1). Later slices move one group at a time.

## 1. The problem

A project reaches the app's state through two restore functions:

* `replay_loaded_project` (`update/project_io/replay/`) — after
  `ClearAll → AllCleared`. Serves a **disk load** (and template instantiate)
  and an undo/redo's **full** restore (the structural fallback,
  `io.restoring_undo` set).
* `try_diff_replay` (`update/project_io/replay_diff.rs`) — an undo/redo's
  **diff** restore, no `ClearAll`, one engine command per changed scalar.

Each domain (tempo events, pool, take lanes, …) was restored by a branch in
each function, and the two functions ran them in different orders. The epic
replaces the branches with per-domain `Reconcile` impls that one driver runs,
in one order, from both paths.

## 2. The trait

```rust
// update/project_io/reconcile/mod.rs
pub enum Origin { DiskLoad, UndoFull, UndoDiff }

pub struct ReconcileCtx<'a> {
    pub origin: Origin,
    /// The project directory relative paths resolve against:
    /// `LoadedProject::project_dir` on the full paths, the live
    /// `io.project_path` on the diff path (`None` for an untitled project).
    pub project_dir: Option<&'a Path>,
}

pub(crate) trait Reconcile {
    const NAME: &'static str;
    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>,
                 new: &ProjectFile, ctx: &ReconcileCtx<'_>);
}
```

**`old`** is `Some(current)` only on `UndoDiff` (the file `try_diff_replay`
already builds for `structurally_compatible`). On the full paths it is
`None`: by the time `AllCleared` arrives there is no pre-clear file to hand
over, and building one would cost a `build_project_file` per undo for
nothing. What the full undo *does* keep is live state that survives
`ClearAll` — freeze statuses, the reference monitor, the derived-clip
counter. Those are read from `r` under `Origin::UndoFull`, not from `old`.
That is why `origin` has three values rather than "old empty / old
present": the slow-path undo is not a disk load with `old = empty`.

**Unit structs + associated fns, not `&self` methods.** A domain has no
per-instance state, so a `self` would be noise. The driver table stores
`fn` pointers made from the impls by a `const fn domain::<D>(stage)`, so
the impl is still where the name and the body live together.

**`restoring_undo` folds into the ctx now, as far as it can.** Every
*read* of the flag inside `replay_loaded_project` becomes a read of
`ctx.origin` (built once at the top of the replay from the flag). The flag
itself stays: it is the only thing that carries "this `ClearAll` is an undo"
across the asynchronous gap to `AllCleared`, where `all_cleared` still reads
and clears it for the disk-load tail (patch resend, scroll reset, relink
modal, job completion). A later slice can move that tail into a domain and
the field into `pending_load`'s payload (`FU-A7a` wants the same).

## 3. The driver and the fixed order

One table, `reconcile::DOMAINS`, lists every migrated domain once, in the
order both paths run them. Each entry also names a **stage**:

```rust
pub enum Stage { Timeline, Content }
pub(crate) fn reconcile_stage(r, stage, old, new, ctx)
```

`reconcile_stage` runs the table's entries of that stage, in table order.
Stages exist only because the not-yet-migrated inline code still sits
between domains on the full path (tempo must be restored before tracks and
clips are replayed; the pool after the clips it counts). Each path calls
the stages in the same sequence, at the point in its own inline code where
that stage is valid. The table is sorted by stage (a test checks it), so
"the concatenation of the stages each path ran" equals the table.

As later slices migrate the inline code between two stages, the stages
become adjacent on both paths and merge; when everything is migrated there
is one stage, one `reconcile_all`, and `structurally_compatible` / the
`ClearAll` fallback can go (roadmap group 6).

**Test hook.** The driver records `(origin, name)` for each domain it runs in
`io.reconcile_trace`, cleared at the start of each restore. The guard test
(`tests/io/reconcile_order.rs`) drives a disk load, a full undo and a diff
undo and asserts each trace equals the table, with the right origin. A
domain that one path restores inline, or that a path runs out of order,
fails it.

## 4. Group (1): what moved and where

Group (1) are app-side domains restored whole on both paths. Their impls
ignore `old`.

| Domain | Stage | Body | Origin-dependent? |
|---|---|---|---|
| `TempoEvents` | Timeline | `restore_tempo_events` + `rebuild_and_send_tempo` | no |
| `ChordTrack` | Timeline | `ProjectFile.chord_track.to_chord_track()` | no |
| `Markers` | Timeline | `ArrangementMarkers::from(arrangement_markers)` | no |
| `Pool` | Content | clear assets, re-add (flag missing against `ctx.project_dir`), recompute usage; `ReserveAssetIds` | the engine reservation is sent only after a `ClearAll` (as before: the diff path never sent it; the engine allocator is monotonic within a session) |
| `Quantize` | Content | `restore_quantize` | no |
| `Performance` | Content | `restore_performance` | no |
| `TrackGroups` | Content | `restore_track_groups` (incl. STATE-04 counter bump) | no |
| `TakeGroups` | Content | clear, then `replay_take_groups` | full paths `clear()` (drops the old project's peak cache), diff path `clear_for_snapshot()` (keeps it, ba #1400) |

Deleted: `replay_diff::{apply_tempo, apply_track_groups, apply_markers,
apply_pool, apply_take_groups}` and their inline calls, the chord/marker/
tempo lines in `replay_globals`, the five restore calls at the end of
`replay_loaded_project`, and the `take_groups.clear()` in `wipe_registry`
(moved into `TakeGroups`; nothing between the wipe and the Content stage
reads or writes `r.take_groups`).

`restore_pool` (now a wrapper over `restore_pool_assets`, which takes the
optional dir and the reserve flag) / `restore_quantize` / `restore_performance` stay as the
shared bodies because `test_support` calls them directly.

### Where the stages sit

Full path (`replay_loaded_project`):

```
SetProjectDir
replay_globals:  transport scalars, UI reset, compose load, drum patterns,
                 SetBpm,
                 ── Stage::Timeline (tempo events, chord track, markers) ──
                 trim_chords_to_sections, SetTimeSignature, metronome, …
wipe_registry, tracks/busses/master/sends/sidechain, audio + MIDI clips,
vocal (derived clips), plugin chains, references,
── Stage::Content (pool, quantize, performance, track groups, take groups) ──
automation lanes, freeze
```

Diff path (`try_diff_replay`):

```
global, tracks, busses, sends, sidechain, master, plugin blobs + params,
audio clips, MIDI clips, compose (derived clips, lyrics), references,
freeze, external instruments,
── Stage::Timeline ──
── Stage::Content ──
automation lanes, resort, vocal audio clip map
```

## 5. Ordering differences found, and why they don't matter

Before this slice the two paths ran group (1) in these orders:

* full: markers, chord, tempo (all in `replay_globals`); … references,
  pool, quantize, performance, track groups, take groups
* diff: track groups, markers, pool, take groups, quantize, performance,
  references, chord, freeze, external instruments, tempo

Moves this slice makes, each checked against every reader in between:

1. **Full path, markers + chord track** move from before
   `restore_drum_patterns` to after `SetBpm`. Nothing in between (drum
   pattern restore, the focus fix-up, `SetBpm`) reads either.
2. **Full path, tempo events** move from before `SetBpm` to after it. The
   `SetBpm` payload is `transport.bpm`, not the events, and
   `rebuild_and_send_tempo` already ran after `SetBpm`, so the engine sees
   the same `SetBpm, SetTempoEvents` sequence. `trim_chords_to_sections`
   (reads the meter via the tempo map) still runs after.
3. **Diff path, tempo events** do not move relative to the inline code (still
   after external instruments, before automation lanes). Moving them early,
   as on the full path, would have changed which tempo map
   `restore_derived_clips`'s legacy rebuild and the engine's clip commands
   see — a behaviour change, left for roadmap group (4) (globals/transport),
   which is where the two paths' tempo positions should converge (tempo
   before clips, as on the full path).
4. **Diff path, chord track + markers** move later, past references, freeze
   and external instruments. None of those reads either.
5. **Diff path, the Content domains** move from between compose and
   references to after tempo. References, freeze, external instruments and
   the tempo rebuild read none of pool / take groups / quantize /
   performance / track groups; the Content domains read none of what those
   write (`recompute_pool_usage` reads `r.clips`' asset refs, set by
   `apply_audio_clips` earlier). The engine sees `RestoreTakeGroups` /
   `LoadTakeClipFromWav` after `SetTempoEvents` and the freeze/external
   commands rather than before; the engine handles those independently.
   This also puts Content *after* references on both paths, as the full
   path always had it.
6. **Full path, take-groups clear** moves from `wipe_registry` into
   `TakeGroups` (see §4).

Inside the Content stage the table keeps the full path's order (pool,
quantize, performance, track groups, take groups); on the diff path those
five are independent.

## 6. What group (2) needs from the trait

Group (2) — automation lanes, derived clips, external instruments,
references, freeze (FU-A4a), missing-plugins — has origin-dependent bodies:

* **Live state across `ClearAll`.** Freeze statuses, the reference monitor
  and the derived-clip counter must be read *before* the full path's wipe
  and used after. Today `replay_loaded_project` captures them into locals
  (`derived_counter_floor`, the `r.freeze.queue`/`reset` split, the
  `restore_references` monitor source). Either each domain gets a
  `prepare(r, ctx)` hook the driver calls at the top of the replay (before
  anything is wiped) returning a per-domain carry, or the ctx grows a
  `live: LiveCarry` struct the entry points fill. The second is simpler and
  is what A-13b should try first.
* **A stage between clips and the tail.** Derived clips run after MIDI clips
  (both paths), external instruments before lanes, freeze last; lanes after
  external instruments. So group (2) adds stages `Clips` (after replay of
  clips: derived clips) and `Tail` (external instruments, lanes, freeze,
  missing-plugins), and on the diff path `references` joins Content's
  neighbour. Freeze last needs the table to end with it.
* **Engine echo expectations.** External instruments on the full path are
  restored per track inside `replay_track` (after `AddTrack`); a separate
  domain after all tracks must send `SetExternalInstrument` +
  `SetTrackDeviceParams` in a separate pass — check the capture tests in
  `tests/io/replay.rs` pin order-insensitive sets.
* **`project_dir` vs `project_path`.** Freeze needs the live `.rproj` path on
  the undo paths and `loaded.project_dir` on disk load; the ctx already
  carries `project_dir`, and the full path's `live_project_path` becomes a
  second ctx field or part of the live carry.
