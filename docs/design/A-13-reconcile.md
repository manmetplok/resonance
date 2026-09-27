# A-13: the `Reconcile` trait, one domain order for both restore paths

Design for `refactor-intent.md` Epic A item 13 (`arch-migration-plan.md`
ARCH-01 step 3, "A-13 roadmap" in the A-7 progress note). Written against
master `d6d89413`. This document covers the trait, the driver, and the first
slice (A-13a, roadmap group 1); §7 records the second (A-13b, group 2,
written against master `c325335a`), §8 the third (A-13c, group 4, against
master `d538d5cf`; group 3 waits for D-2/D-3), §9 the fourth (A-13d, group
5, against master `85b38b32`), §10 the fifth (A-13e, group 3, against
master `1853dd1e`), §11 the sixth (A-13f, group 6 step 1, against
master `fde89f24`), §12 the seventh (A-13g, group 6 step 2, against
master `645d49e1`), §13 the eighth (A-13h, group 6 step 3, against master
`91ec9867`), §14 the ninth (A-13i, group 6 step 4, against master
`815af005`), §15 the last (A-13j, delete the fallback, against master
`73bd9ad1`). Sections 1–14 describe the code as each slice left it;
`Origin::UndoFull`, `try_diff_replay` and `io.restoring_undo` in them
are gone since A-13j.

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

## 7. Group (2): A-13b

### The live carry

```rust
pub struct ReconcileCtx<'a> { origin, project_dir, pub live: LiveCarry<'a> }

#[derive(Clone, Copy, Default)]
pub struct LiveCarry<'a> {
    pub project_path: Option<&'a Path>,     // live .rproj path (freeze caches)
    pub derived_counter_floor: Option<u64>, // None on a disk load
}
```

Each entry point fills it at its top, before anything is restored. It holds
only what the restore itself **overwrites before the domain that needs it
runs**:

* `project_path` — the full replay `take()`s `io.project_path` (the
  `AllCleared` handler puts it back after), so `Freeze` could not read it
  from `r`. The diff path clones it; there it equals `project_dir`.
* `derived_counter_floor` — `ComposeState::load_from_project` (in
  `replay_globals` / `apply_compose`) resets the counter before
  `DerivedClips` runs. Captured at the top of `try_diff_replay` now rather
  than inside `apply_compose`; nothing in between touches the counter.

The other two items §6 listed turned out not to need carrying: live state a
restore does not overwrite before its domain runs stays in `Resonance` and is
read there under the origin.

* **Freeze statuses.** Nothing in either restore reads or writes `r.freeze`
  before `Freeze` runs (checked: every `.freeze` reader is a message gate,
  an engine-event handler or the serializer). The full path's top-of-replay
  `freeze.reset()` (disk load) / `queue = None` (undo) moved into the domain;
  `apply_freeze_restore` clears the queue itself.
* **Reference monitor.** `restore_references` already `mem::take`s
  `r.reference.monitor` itself and nothing before it touches that.

So "carry" is the exception, not the rule: the next group should only add a
field when a restore step between the top and the domain clobbers the value.

### Stages and the table

`Stage` is now `Timeline, Clips, Content, Tail`. The table (pinned by
`reconcile_order::the_table_is_the_agreed_order`):

| Stage | Domains |
|---|---|
| Timeline | tempo_events, chord_track, markers |
| Clips | derived_clips |
| Content | **references**, pool, quantize, performance, track_groups, take_groups |
| Tail | **external_instruments**, **automation_lanes**, **missing_plugins**, **freeze** |

| Domain | Body by origin |
|---|---|
| `DerivedClips` | `restore_derived_clips(file, echoes_in_flight = UndoDiff, live.derived_counter_floor)` |
| `References` | DiskLoad: `restore_references(File)`; UndoFull: `restore_references(Live)`; UndoDiff: `reconcile_references` |
| `ExternalInstruments` | `restore_external_instruments(file, after_clear_all)` — one body now (below) |
| `AutomationLanes` | `restore_automation_lanes` — origin-independent |
| `MissingPlugins` | DiskLoad: `reset`; UndoFull: `dismiss`; UndoDiff: nothing (re-adds no plugin — as before) |
| `Freeze` | DiskLoad: `freeze.reset()` + `rehydrate_frozen_tracks(project_dir)`; undo (both): `apply_freeze_restore(tracks, live.project_path)` |

`restore_external_instruments` absorbed `replay_track`'s block: with
`after_clear_all` it drops the app map (no engine traffic; `ClearAll` emptied
it) and sends no `SetTrackDeviceParams` for a track with no device — what
`replay_track` did, pinned by
`legacy_external_track_without_device_field_loads_and_sends_no_params`.
Without it (diff path) stale tracks get `ClearExternalInstrument` + an empty
param map as before. Tracks are asserted in **file order** (the diff path
iterated a `HashMap`). It is keyed by the file's track id; `replay_track`
used the remapped id of a legacy colliding sub-track, but sub-tracks cannot
be made external, so the two never differ in practice. The disk-only
`ResendExternalInstrumentPatches` stays in `all_cleared`'s disk tail.

Where the stages sit now:

```
Full path (replay_loaded_project)
  SetProjectDir, replay_globals [.. SetBpm, ── Timeline ──, chord trim, ..],
  wipe_registry, tracks/busses/master/sends/sidechain, audio + MIDI clips,
  replay_vocal [── Clips ──, vocal audio clip map], finalize_plugin_chains,
  ── Content ── ── Tail ──

Diff path (try_diff_replay)
  global, tracks, busses, sends, sidechain, master, plugin blobs + params,
  audio clips, MIDI clips, compose (sections, drum patterns, lyrics),
  ── Timeline ── ── Clips ── ── Content ── ── Tail ──,
  resort, vocal audio clip map
```

On the diff path the four stages are now adjacent (there is no inline code
left between them).

### Ordering changes (each checked against every reader in between)

Full path:

1. **External instruments** leave `replay_track`: `SetExternalInstrument` /
   `SetTrackDeviceParams` go out after every track, bus, plugin, clip,
   reference and Content domain instead of right after each track's
   `AddTrack` + plugins. Still before the lanes and before the disk-load
   `ResendExternalInstrumentPatches`. The engine stores the config in a
   per-track map (`set_external_instrument_in_place` only needs the track to
   exist) read at render time; app-side nothing between `wipe_registry` and
   the Tail reads `r.external_instruments`. The capture tests counted
   commands per track rather than position, except the fixed-point test's
   "exactly once", which holds. New guard:
   `the_full_path_sends_external_config_after_the_tracks_and_before_the_lanes`.
   The `external_instruments.clear()` in `wipe_registry` moved into the
   domain.
2. **Missing-plugin reset/dismiss** moves from the top of the replay to the
   Tail. The refusals that raise the warning are engine events handled after
   the replay returns (`engine_events::plugins`), and nothing in the replay
   reads the warning.
3. **Freeze prep** (`reset` / `queue = None`) moves from the top of the
   replay into `Freeze` (see the carry above).

Diff path:

4. **Derived clips** move from inside `apply_compose` (before lyrics,
   references, freeze, external instruments and tempo) to after `Timeline`.
   None of those read the map or allocate a derived id. The legacy
   positional rebuild reads the tempo map, so it now sees the *target*'s
   (the full path's behaviour) — but undo snapshots always carry
   `derived_clips` (`build_project_file` writes `Some`), so it is not
   reached from the diff path.
5. **References** move from after `apply_compose` to the head of Content
   (after Timeline and Clips). Freeze, external instruments, tempo and the
   derived map read none of it.
6. **Freeze** moves from after references to the end of the Tail;
   **external instruments** from before Timeline to the head of the Tail.
   Neither reads the other or anything in between; the engine sees
   `SetExternalInstrument` and a retired freeze's detach commands after
   `SetTempoEvents` and the Content commands instead of before, and treats
   them independently.

No capture test depended on any of these positions; every existing guard
passed unchanged at each commit.

### FU-A4a

Fixed after A-13b (branch `fix/FU-A4a`). `Freeze`'s undo arm passes
`origin.after_clear_all()` to `apply_freeze_restore`; for each track the
target has frozen (`Frozen` or `Stale`) whose engine source is not attached
— every one after a `ClearAll`, on the diff path those whose live status was
not frozen — `reconcile_freeze_statuses` decodes and attaches the cache
through `attach_freeze_cache` (shared with `rehydrate_frozen_tracks`),
downgrading an undecodable or missing one to `Stale`. A track frozen
throughout a diff restore is not re-decoded. Baselines are untouched
(FU-H2b). Guards: `plugins::freeze_persist::undo_reattach`, attach counts in
`undo_snapshot_fixed_point`.

### What groups (3) and (4) need

* **(3) Routing — sends, sidechain routes.** Diff-shaped on both paths
  (`old` is finally used): after a `ClearAll`, `old = None` means "empty the
  mirror, send everything"; the `aux.sends.clear()` / `sidechain.clear()`
  in `wipe_registry` move into the domains as `TakeGroups`' clear did. The
  full path validates endpoints (drops and warns); the diff path relies on
  `structurally_compatible` — keep the validation in the shared body.
  **Stage:** a new `Routing` between `Timeline` and `Clips` fits both paths
  without reordering anything else: full path right after
  `replay_tracks_and_busses` (sidechain needs the master chain's plugins,
  so after `replay_master`), diff path right after `Timeline`. The diff
  path's sends then go out after the clip moves instead of before them —
  independent on the engine. Mind the diff path's "removals first" rule.
* **(4) Globals / transport / compose sections.** `replay_globals` sends
  every scalar, `apply_global` only changed ones; the UI reset
  (`selected_clip`, drags, confirm modals) is full-path only;
  `restore_drum_patterns`' `clear_when_empty` flag differs (`false` on the
  full path, `true` on the diff path) — a real origin rule to keep. This is
  where the diff path's tempo should converge to "before the clips": a
  `Globals` stage before `Timeline` on both paths, which on the diff path
  means moving Timeline ahead of `apply_audio_clips` / `apply_midi_clips`
  (a real engine-order change: clip commands would see the target tempo
  map — the reason A-13a left it). `load_from_project` resets the derived
  map and counter; the floor is already carried, so moving the compose load
  is safe for `DerivedClips` as long as it stays before `Clips`.
* `ctx.project_dir` and `ctx.live.project_path` are the same value on the
  diff path; once the full path no longer `take()`s `io.project_path`
  (FU-A7a's `pending_load` payload rework) the carry field can go.

## 8. Group (4): A-13c

Group (3) (sends, sidechain routes) is skipped for now: D-2/D-3 is
re-keying send and bus ids concurrently. This slice moves group (4).

### What `replay_globals` / `apply_global` actually covered

| Piece | Full path (`replay_globals`) | Diff path |
|---|---|---|
| bpm, time sig, metronome, master volume, MIDI clock in/out, loop range | set + sent, all | `apply_global`: set + sent when changed |
| playhead = 0 | yes | no |
| `loop_range_set = loop_enabled` | end of `replay_vocal` | inside the loop-changed branch |
| selected clip/plugin, clip drag/trim, delete-track/quit confirm | reset | kept |
| sections (`load_from_project`) | yes | `apply_compose`, after the MIDI clips |
| drum-pattern bank | `restore_drum_patterns(.., false)` + drum-roll focus | `(.., true)`, focus kept |
| chord trim to sections | after `Timeline` | no |
| lyric side-table | per clip in `replay_midi_clips` | `apply_compose`, after the MIDI clips |

### Domains and stages

`Stage` is now `Globals, Timeline, Clips, Content, Tail`; 20 domains
(`reconcile_order::the_table_is_the_agreed_order`). New rows (code
`reconcile/globals.rs`):

| Stage | Domain | Body by origin |
|---|---|---|
| Globals | `transport` | each scalar set + sent when `old` differs — every one when `old = None`. After `ClearAll`: playhead 0. `loop_range_set` follows the loop whenever the loop is restored. The first domain to read `old`. |
| Globals | `transient_ui` | after `ClearAll`: the UI reset; diff: nothing |
| Globals | `compose_sections` | `load_from_project` — all origins |
| Globals | `drum_patterns` | `restore_drum_patterns(clear_on_empty = UndoDiff)`; after `ClearAll` also the drum-roll focus |
| Timeline (last) | `section_chord_trim` | after `ClearAll`: `trim_chords_to_sections`; diff: nothing |
| Clips (first) | `clip_lyrics` | clear + `restore_clip_lyrics` padded to the restored note counts — all origins |

The explicit origin branches, and why each is kept:

* **`drum_patterns`' `clear_on_empty`.** Kept as it was. On the diff path
  it is close to dead: `structurally_compatible` requires equal
  `drum_patterns` id sets, so an empty target means the live bank is empty
  too and the clear is a no-op. The exception is an old snapshot that only
  has `drum_groups`, which takes the promotion branch anyway. Kept rather
  than proven away. (Live on the diff path since A-13g, §12.)
* **`transient_ui`, drum-roll focus, playhead.** Full path only, as before.
  The diff path never removes the entities they name.
* **`section_chord_trim`.** Full path only, as before. A diff target is a
  snapshot of live state that every edit already kept trimmed. Trimming it
  would also make the restore differ from the snapshot (fixed-point).
* **`transport` changed-only.** On the diff path, scalars are compared
  against `old`. `old` is `build_project_file(r)`, so "unchanged" means
  "equals live". The skipped sets were no-ops. The skipped sends are what
  the diff path never sent.

### Where the stages sit now

```
Full path (replay_loaded_project)
  vocal_audio.clear, SetProjectDir,
  ── Globals ── ── Timeline ──,
  wipe_registry, tracks/busses/master/sends/sidechain, audio + MIDI clips,
  replay_vocal [── Clips ──, vocal audio clip map], finalize_plugin_chains,
  ── Content ── ── Tail ──

Diff path (try_diff_replay)
  ── Globals ── ── Timeline ──,
  tracks, busses, sends, sidechain, master, plugin blobs + params,
  audio clips, MIDI clips,
  ── Clips ── ── Content ── ── Tail ──,
  resort, vocal audio clip map
```

`replay_globals`, `apply_global` and `apply_compose` are deleted. Both paths
now run the same stage sequence with inline code in the same two gaps:
entities + clips between `Timeline` and `Clips`, and (full path only)
plugin-chain finalisation between `Clips` and `Content`.

### Tempo convergence (diff path): done

`SetTempoEvents` (and the chord track / markers) on the diff path moved
from after the MIDI clips to before the first track command. That matches
the full path. Checked against every reader between the old and new
positions:

* **Engine.** The commands the diff path sends in between are track, bus,
  send, sidechain, master and plugin bypass scalars, `LoadPluginState`,
  param sets, `MoveClip` / `TrimClip` / `SetClipFade` / `SetClipGain`, and
  `DeleteMidiClip` + `LoadMidiClipDirect` / `TrimMidiClip` /
  `MoveMidiClip`. None of their handlers reads `ctx.tempo_map`. MIDI clips
  store ticks and a start sample; the tick→sample projection happens at
  render time (`midi/outbound.rs`). The engine's tempo-map readers are:
  quantize / groove / extract-groove handlers, the count-in in
  `handle_record`, the control loop's per-tick `sync_bpm_at`, and the
  render, bounce, freeze and audition paths. None is on the restore path.
* **App.** `apply_tracks` … `apply_midi_clips` read none of `tempo_map`,
  `tempo_events`, `signature_events`, `chord_track`, `markers` or
  `compose`. `restore_derived_clips`' legacy positional rebuild (the only
  tempo-map reader in a restore) is in `Clips`, after `Timeline` on both
  paths either way. Undo snapshots always carry `derived_clips`, so the
  diff path does not reach it.

So the only difference is *when* the engine holds the target tempo map
relative to the clip commands. It holds it before them now, as after a
full replay. Guard:
`reconcile_order::a_diff_undo_sends_tempo_before_the_clips_and_only_changed_scalars`
(it fails on the A-13b order).

### Other ordering changes (each checked against every reader in between)

Full path:

1. **`SetTimeSignature`, `SetMetronomeEnabled`, `SetMasterVolume`,
   `SetMidiClock{Output,Input}`, `SetLoopRange`** go out before
   `SetTempoEvents` instead of after. `SetBpm` still precedes it (the
   events' first point must win the engine map's `bpm`).
   `SetTimeSignature` writes the map's fallback `numerator`/`denominator`.
   `rebuild_bar_table` reads those only when there are no signature
   points, and `restore_tempo_events` always installs at least one. So the
   two commute. The rest are independent atomics and ports. The guard
   `the_full_path_sends_bpm_then_tempo_events_then_meter` became
   `the_full_path_sends_the_transport_scalars_then_tempo_events`.
2. **`SetBpm`** goes out before the section load and drum-bank restore
   instead of after. Neither sends anything nor reads the transport.
3. **`loop_range_set`** is set in `transport` instead of at the end of
   `replay_vocal`. Its only readers are message handlers and views.
4. **Lyrics** move from inside `replay_midi_clips` to the head of `Clips`.
   `replay_vocal` opens with that stage, so nothing runs in between.

Diff path:

5. **Transport scalars:** `SetLoopRange` now follows the MIDI-clock pair
   instead of preceding it (full-path order). These are independent engine
   settings.
6. **Sections + drum bank** move from after the MIDI clips to the head.
   The derived-counter floor was already captured before them. Nothing
   from `apply_tracks` to `apply_midi_clips` reads compose state.
7. **Lyrics** move from before `Timeline` to the head of `Clips`, still
   after `apply_midi_clips` (note counts) and before `DerivedClips`.
   Nothing in between reads them.

Every existing guard passed unchanged, apart from the renamed full-path
order test above.

### What's next

* **(3) Routing.** A `Routing` stage between `Timeline` and `Clips` still
  fits. Full path: after `replay_master` (sidechain needs the master
  chain's plugin ids). Diff path: after `apply_master`, before the plugin
  blobs. Shapes are unchanged there, so the diff path's current
  "sends after busses" rule holds. `wipe_registry`'s `aux.sends.clear()` /
  `sidechain.clear()` move into the domains. Model the domains on
  `transport`'s `old = None` ⇒ "send everything" shape.
* **(5) Clips.** `apply_audio_clips` / `apply_midi_clips` vs
  `replay_audio_clips` / `replay_midi_clips` become a `Clips`-stage head.
  Full path: load everything. Diff path: move/trim/reload by diff against
  `old`. The clip-id sets are equal on the diff path. The vocal audio clip
  map rebuild, still inline at the end of both paths, can follow as the
  last `Clips` domain once the diff path's copy moves ahead of the
  registry resort. Lyrics already moved here.
* **(6) Structural.** Tracks, busses, master, plugins; then
  `structurally_compatible` and the `ClearAll` fallback go, and
  `wipe_registry` / `finalize_plugin_chains` fold into those domains.

## 9. Group (5): A-13d

### Domains

`Clips` now opens with the clips themselves and closes with the vocal
audio-clip map; 23 domains (`reconcile_order::the_table_is_the_agreed_order`).
Code: `reconcile/clips.rs`.

| Stage | Domain | Body by origin |
|---|---|---|
| Clips (1st) | `audio_clips` | after `ClearAll`: empty `r.clips`, then per clip `LoadClipFromWav` + `SetClipFade` / `SetClipGain` when non-default + push the mirror (with `asset_ref`). Diff: per clip `TrimClip`, else `MoveClip`, then `SetClipFade` / `SetClipGain` when changed; mirror updated in place. |
| Clips (2nd) | `midi_clips` | after `ClearAll`: empty `r.midi_clips`, then `LoadMidiClipDirect` + push. Diff: notes (vs the live mirror) or length changed → `DeleteMidiClip` + `LoadMidiClipDirect`; else `TrimMidiClip`; else `MoveMidiClip`; mirror updated in place. |
| Clips | `clip_lyrics`, `derived_clips` | unchanged (A-13c, A-13b) |
| Clips (last) | `vocal_audio_clips` | `rebuild_vocal_audio_clips` from `r.clips`, paths `ctx.project_dir.join(audio_file)` — all origins |

`ReconcileCtx` gains `midi_notes: &HashMap<ClipId, Vec<MidiNote>>`, the
target `LoadedProject`'s notes: the `ProjectFile` names a clip's notes only
by its `.mid` file. Both entry points already held the `LoadedProject`. Notes
are plain `Vec<MidiNote>` clones as before (ARCH-09's `Arc` sharing has not
landed). The notes compare on the diff path is still against the live
mirror, not `old`, because the snapshot file carries no notes.

The per-path bodies are the old ones, moved: `replay_audio_clips`,
`replay_midi_clips`, `replay_vocal`, `apply_audio_clips` and
`apply_midi_clips` are deleted, and so is the inline map rebuild at the end
of `try_diff_replay`. `wipe_registry`'s `r.clips.clear()` /
`r.midi_clips.clear()` moved into the domains' `old = None` arm, as
`TakeGroups`' clear did. Nothing between the wipe and `Clips` (tracks,
busses, master, outputs, sends, sidechain routes) reads either mirror.

### What the diff path relies on

`structurally_compatible` still gates it: equal audio and MIDI clip-id sets,
and an audio clip's `audio_file` and `total_frames` unchanged. So the diff
arm never loads or deletes a clip, and never reloads a WAV. A clip id in
`new` but not in `old` is skipped (defence in depth), as before. Replacing
that gate with "load the added, delete the removed" is group (6)'s job,
once tracks can be added on the diff path too.

### Things checked that did not move

* **Clip WAVs (V6 / FU-V5b).** `PersistClipWavs` is sent by
  `snapshot_for_undo`, not by the restore. The full path's `LoadClipFromWav`
  still reads `audio/clip_<id>.wav` under the project dir.
* **Clip ids (STATE-08, FU-A6a).** The engine bumps its allocator past each
  loaded id in `LoadClipFromWav` / `LoadMidiClipDirect`; the same commands
  go out with the same ids.
* **Pool.** `Pool` (Content) counts `r.clips`' asset refs, which
  `audio_clips` sets on both arms. Still Clips before Content.
* **Derived-counter floor.** `DerivedClips` still runs after both clip
  domains (it filters against `r.midi_clips` and reserves past them), and
  `vocal_audio_clips` after it (it reserves past the audio clip ids). The
  floor is carried in `LiveCarry` as before.
* **Take groups.** Take clips live in the engine's take store
  (`RestoreTakeGroups`, Content), not in `r.clips`; untouched.

### Ordering changes (each checked against every reader in between)

Full path: none. The domains run where `replay_audio_clips`,
`replay_midi_clips` and `replay_vocal` ran, in the same order, with the same
commands; only the two mirror clears move later (from `wipe_registry` to
the top of their domains, see above).

Diff path:

1. **Audio and MIDI clips** join the `Clips` stage; they were the two calls
   right before it. No change.
2. **The vocal audio-clip map** moves from after `Tail`, the track/bus
   resort, `rebuild_output` and `refresh_track_count` to right after
   `DerivedClips`, which is the full path's position. The rebuild reads
   `r.clips` (`Content` and `Tail` don't write it), the vocal track set
   (a set, so the resort cannot change it; track types are structural),
   the placements (`Globals`) and the tempo map (`Timeline`). It writes
   `vocal_audio.clips` and reserves `next_derived_clip_id`. Nothing in
   `Content` or `Tail` reads either: the map's readers are the vocal install
   / tear-down handlers and the clip-deleted event; the counter's are
   `DerivedClips` and the allocator. The freeze fingerprint reads the MIDI
   clips and the lyric side-table, not the map. It sends no engine command.
   Guard: `reconcile_order::a_diff_undo_rebuilds_the_vocal_audio_clip_map_from_the_target`
   (fails if the diff arm skips the rebuild); the trace tests pin the
   position.

Every existing guard passed unchanged.

### Where the stages sit now

```
Full path (replay_loaded_project)
  vocal_audio.clear, SetProjectDir,
  ── Globals ── ── Timeline ──,
  wipe_registry, tracks/busses/master/outputs/sends/sidechain,
  ── Clips ──, finalize_plugin_chains,
  ── Content ── ── Tail ──

Diff path (try_diff_replay)
  ── Globals ── ── Timeline ──,
  tracks, busses, sends, sidechain, master, plugin blobs + params,
  ── Clips ── ── Content ── ── Tail ──,
  resort
```

The only inline code left between `Timeline` and `Clips` is entities
(tracks, busses, master, plugins, with the full path's track outputs) and
routing (sends, sidechain routes).

### What's next

* **(3) Routing.** Unchanged from §8: a `Routing` stage between `Timeline`
  and `Clips`. Full path: after `replay_master`. Diff path: after
  `apply_master`, before the plugin blobs, which puts sends after the
  master instead of before it (not checked yet; the slice must show the
  engine treats the two independently). Model the
  domains on `audio_clips`' shape: `old = None` clears the mirror and sends
  everything, which also moves `aux.sends.clear()` / `sidechain.clear()` out
  of `wipe_registry`.
* **(6) Structural.** Tracks, busses, master, plugins (blobs + params, and
  the full path's `finalize_plugin_chains` order fix-up). Once entities
  have add/remove arms on the diff path, the clip domains' diff arm grows
  "load what `old` lacks, delete what `new` lacks" (the full arm's
  `load_audio_clip` / `load_midi_clip` are the add bodies), and
  `structurally_compatible` and the `ClearAll` fallback can go. The
  full path's top-of-replay `vocal_audio.clear()` (lyrics, render epochs)
  can fold into `ClipLyrics` / `VocalAudioClips` then. `ctx.midi_notes`
  stays: the file will still not carry notes.

## 10. Group (3): A-13e

### Domains

`Stage` is now `Globals, Timeline, Routing, Clips, Content, Tail`; 25
domains (`reconcile_order::the_table_is_the_agreed_order`). Code:
`reconcile/routing.rs`.

| Stage | Domain | Body by origin |
|---|---|---|
| Routing (1st) | `sends` | after `ClearAll`: empty `r.aux.sends` (+ `last_rejection`), then `AddAuxSend` + mirror per send. Diff: `RemoveAuxSend` + mirror drop for every send `old` has and `new` lacks (**removals first**), then per send that is new or changed: `SetAuxSend` when `old` has its id, else `AddAuxSend` (D-2); unchanged sends send nothing. |
| Routing (2nd) | `sidechain_routes` | after `ClearAll`: empty `r.sidechain`, then `SetSidechainRoute` + mirror per route. Diff: `ClearSidechainRoute` for every keyed plugin `new` no longer keys (first), then `SetSidechainRoute` per new or changed route. |

One body validates every edge it sends, on every origin: an unknown source
kind, a missing source track/bus, a missing destination bus (sends) or a
target plugin id absent from the file's chains (routes) drops the edge with
a warning and does not mirror it. On the diff path this is a no-op for any
snapshot the app took: `structurally_compatible` makes the track, bus and
plugin id sets of `old` and `new` equal, and the live mirrors never hold an
edge onto a missing endpoint (deleting an endpoint prunes its edges, ba
#1269). Before, the diff path dropped only an unknown source kind, silently,
and would have sent (and mirrored) an edge onto a missing endpoint that the
engine then rejected.

Deleted: `replay_sends`, `replay_sidechain_routes`,
`saved_plugin_instance_ids` (now `routing::plugin_instance_ids`),
`apply_sends`, `apply_sidechain_routes`; `wipe_registry`'s
`aux.sends.clear()` / `aux.last_rejection = None` / `sidechain.clear()`
moved into the domains' `old = None` arm. Nothing between the wipe and
`Routing` (tracks, busses, master, track outputs) reads either mirror.

### Where the stages sit now

```
Full path (replay_loaded_project)
  vocal_audio.clear, SetProjectDir,
  ── Globals ── ── Timeline ──,
  wipe_registry, tracks/busses/master/outputs,
  ── Routing ── ── Clips ──, finalize_plugin_chains,
  ── Content ── ── Tail ──

Diff path (try_diff_replay)
  ── Globals ── ── Timeline ──,
  tracks, busses, master,
  ── Routing ──,
  plugin blobs + params,
  ── Clips ── ── Content ── ── Tail ──,
  resort
```

The only inline code left between `Timeline` and `Clips` is entities:
tracks, busses, master and (full path) the track outputs; plugin blobs and
params (diff path).

### Ordering changes

Full path: none. `Routing` runs exactly where `replay_sends` /
`replay_sidechain_routes` ran (the tail of `replay_tracks_and_busses`, after
the master chain and the `SetTrackOutput`s, before `Clips`), with the same
commands. Guard:
`the_full_path_sends_routing_after_the_master_chain_and_before_the_clips`.

Diff path: sends and key routes move from between `apply_busses` and
`apply_master` to after `apply_master` (still before the plugin blobs and
params, still sends before routes). What `apply_master` sends is
`SetMasterFxBypass` and `SetPluginBypass` for master slots; app-side it
writes `r.master_fx_bypassed` and the master slots' `plugin_name` /
`bypassed`. Checked against the engine handlers (all on the one control
thread, FIFO):

* `handle_add_aux_send` / `handle_set_aux_send` / `handle_remove_aux_send`
  (`engine/busses.rs`) read `ctx.busses`, `ctx.tracks` (endpoint checks) and
  `state.aux_sends` (id collision, cycle check); they write
  `state.aux_sends` and republish the render snapshot. No plugin, master
  chain or bypass state.
* `sidechain::handle_set` / `handle_clear` read and write only
  `state.sidechain_routes` and its published snapshot. They do not check
  that the plugin exists (a route onto a plugin with no key port is stored
  harmlessly; the mixer decides at render time).
* `handle_set_master_fx_bypass` (`engine/master.rs`) writes only
  `shared.master_fx_bypass`; `handle_set_plugin_bypass` reads `ctx.plugins`
  and writes the slot's bypass flag. Neither reads a send or a route.

So the two groups touch disjoint engine state and commute. The echoes
(`AuxSendChanged` / `AuxSendRemoved` / `SidechainRouteChanged` vs
`MasterFxBypassChanged` / `PluginBypassChanged`) update disjoint app
mirrors, so their relative order does not matter either. App-side, the
routing domains read `r.registry` tracks/busses and `new`'s plugin ids,
none of which `apply_master` writes. Guard:
`a_diff_undo_sends_routing_after_the_master_and_before_the_clips` (fails on
the A-13d order, where `SetAuxSend` preceded `SetMasterFxBypass`).

Every existing guard (fixed-point, `undo_restore_flag`, `io::replay`,
`aux_send_*`, `control_sends`, `sidechain_persistence`, `id_allocation`)
passed unchanged.

### What's next: group (6), structural

What is left inline: `wipe_registry`, `replay_tracks_and_busses`
(`replay_track` / `replay_bus` / `replay_master`, each with its plugin chain
via `replay_plugins`, plus sub-track creation, `migrate_old_generate_params`,
the resorts and the `SetTrackOutput`s) and `finalize_plugin_chains` on the
full path; `apply_tracks` / `apply_busses` / `apply_master` /
`push_all_plugin_states` / `apply_all_plugin_params` and the final resort
on the diff path; `structurally_compatible` choosing between them. Proposed
split, one todo each, strictly in order (all touch `replay*/`), and **after
D-4** (which re-keys track add sites in `replay/entity.rs`):

1. **A-13f — entity scalars as domains, shape still gated.** New
   `Stage::Entities` between `Timeline` and `Routing`: `tracks`, `busses`,
   `master`, `track_outputs`, then `plugin_state` (blob + params). Full arm =
   today's `replay_*` bodies (add + every scalar); diff arm = today's
   `apply_*` (changed scalars only). `plugin_state`'s full arm is the blob /
   param-override parking now inside `replay_plugins`, its diff arm
   `push_all_plugin_states` + `apply_all_plugin_params`; the full path's
   `SetPluginBypass` moves with it or stays in the add body (decide by
   echo order: `PluginAdded` must still overwrite the placeholder). Keep
   `structurally_compatible`; `wipe_registry`'s entity clears move into the
   `old = None` arms. Order change to prove: the diff path's plugin
   blobs/params would move before `Routing` (routing touches no plugin
   state — same evidence as above). `finalize_plugin_chains` + the resorts
   become a last `Entities` domain (`entity_order`), full arm sort + index
   rebuild, diff arm resort (moving the diff resort from after `Tail` to
   before `Routing` needs its readers checked: `rebuild_output`,
   `refresh_track_count`, anything in `Clips`..`Tail` iterating
   `registry.tracks` in order). Guards: trace tests, fixed point, a
   command-order test per moved piece.
2. **A-13g — shrink the gate to what the diff arms can't do.** The
   app-side id-set checks in `structurally_compatible` (section
   definitions / placements, drum groups / patterns, track groups,
   markers) cover domains that are restored whole on both paths since
   A-13a/c. Drop them one at a time, each with a diff-undo test across an
   add/remove of that entity that asserts the fixed point. Watch
   `drum_patterns`' `clear_on_empty` (§8) and `DerivedClips` (placements
   key derived clips).
3. **A-13h — add/remove arms for busses and plugin instances.** Diff arms
   gain "add what `old` lacks" (the full arm's add body: `AddBus`,
   `AddPlugin*` with id hint + blob) and "remove what `new` lacks"
   (`RemoveBus`, `RemovePlugin*`, pruning edges first — `Routing` already
   handles edge removal, but it runs after `Entities`, so removals must
   split: a `RoutingRemovals` pre-pass before entity removal, or entity
   removal in a late stage). Reorder = `MovePluginIn*`. Drops the bus and
   plugin checks from the gate.
4. **A-13i — add/remove tracks (incl. sub-tracks, track type change as
   remove + add) and clips.** Track add/remove on the diff path; the clip
   domains' diff arm gains `load_audio_clip` / `load_midi_clip` for ids
   `old` lacks and `DeleteClip` / `DeleteMidiClip` for ids `new` lacks;
   external instruments / freeze / lanes of an added track need their
   `after_clear_all` behaviour per track, not per restore.
5. **A-13j — delete the fallback.** With the gate always true for undo,
   `structurally_compatible`, the undo `ClearAll`, `io.restoring_undo`
   (FU-A7a's `pending_load` payload) and `Origin::UndoFull` go; the
   `live` carry shrinks (`project_path` is `project_dir` once nothing
   `take()`s it). Disk load keeps `ClearAll` + `old = None`. The two
   entry points merge into one `reconcile_all(old, new, ctx)`, and
   `Stage` collapses to the table order. Audible win: no plugin
   re-instantiation on a structural undo.

Each of 3–5 changes engine traffic on structural undo (no `ClearAll`), so
each needs a capture test for the new commands and a fixed-point run over
an add/remove of the entity it covers.

## 11. Group (6), step 1: A-13f

Written against master `fde89f24` (after D-4, A-12e, FU-A6d).

### Domains

`Stage` is now `Globals, Timeline, Entities, Routing, Clips, Content,
Tail`; 31 domains (`reconcile_order::the_table_is_the_agreed_order`). Code:
`reconcile/entities.rs` (the old `replay/entity.rs` bodies plus the diff
arms from `replay_diff.rs`) and `reconcile/plugin_state.rs`.

| Stage | Domain | Body by origin |
|---|---|---|
| Entities (1st) | `tracks` | after `ClearAll`: empty `registry.tracks`, `next_track_order`, `plugin_mirror.index`; bump `next_track_id` past every saved id; per track `replay_track` (add command + every scalar + `AddPlugin` per slot + placeholder slots seeded with name and bypass); then `migrate_old_generate_params`. Diff: `apply_track` — changed scalars only; the mirror takes every field and the slot names. |
| Entities | `busses` | after `ClearAll`: empty `registry.busses`, `next_bus_order`; `replay_bus` per bus. Diff: `apply_bus`. |
| Entities | `master` | after `ClearAll`: `SetMasterFxBypass` + `AddPluginToMaster` per slot. Diff: `SetMasterFxBypass` when changed, slot names. |
| Entities | `track_outputs` | `SetTrackOutput` — after `ClearAll` for every track routed to a bus; diff for every track whose `output_bus` differs from `old`'s (incl. back to the master). The mirror (`TrackState::output`) stays with `tracks`. |
| Entities | `plugin_state` | three phases on every origin: **blobs** (`LoadPluginState` + `state_cache`; after `ClearAll` all of them, diff only when not `Arc::ptr_eq` with the live cache — FU-A2b), **bypass** (after `ClearAll` `SetPluginBypass` per bypassed slot; diff `apply_plugin_bypass` per changed slot), **params** (after `ClearAll` parked in `pending_plugin_param_overrides`; diff `apply_all_plugin_params`). |
| Entities (last) | `entity_order` | every origin: `resort_tracks`, `resort_busses`, `rebuild_output`, `refresh_track_count`. After `ClearAll` also each chain sorted into the target file's saved order, then `rebuild_plugin_index`. |

Deleted: `wipe_registry`, `replay_tracks_and_busses`, `SavedPluginOrder`,
`finalize_plugin_chains` (`replay/mod.rs`); `apply_tracks`,
`apply_busses`, `push_all_plugin_states`, `push_plugin_states` and the
trailing resort (`replay_diff.rs`); `reconcile_stage` (replaced by
`reconcile_all_stages`). `ReconcileCtx` gains `plugin_states`, the target
`LoadedProject`'s blobs, for the same reason it carries `midi_notes`: the
`ProjectFile` does not hold them. `wipe_registry`'s entity clears moved
into the `old = None` arms (`master`'s is an assignment).
`structurally_compatible` and the `ClearAll` fallback stay.

### Where the stages sit now

```
Full path (replay_loaded_project)
  vocal_audio.clear, SetProjectDir, reconcile_all_stages(old = None)

Diff path (try_diff_replay)
  structurally_compatible?, reconcile_all_stages(old = Some(current))
```

No per-path restore code is left between two stages, so the stage calls
became one `reconcile_all_stages` (the table in order). The disk-load
tail stays in `all_cleared`.

### Proof 1: the diff path's plugin blobs and params move before `Routing`

Diff order before: entity scalars (incl. `SetPluginBypass`), `Routing`
(`RemoveAuxSend` / `SetAuxSend` / `AddAuxSend`, `ClearSidechainRoute` /
`SetSidechainRoute`), `LoadPluginState`, `SetPluginParam`. Now the blobs
and params precede `Routing`.

* **Engine.** `handle_load_plugin_state` (`engine/plugins.rs`) reads and
  locks only `ctx.plugins[instance_id]` and reloads that instance
  (re-queued on `cmd_tx_retry` if the audio thread holds the lock, as
  before). `handle_set_plugin_param` likewise touches one instance. The
  routing handlers (§10: `handle_add/set/remove_aux_send`,
  `sidechain::handle_set/clear`) read `ctx.tracks` / `ctx.busses` and
  write `state.aux_sends` / `state.sidechain_routes` only;
  `sidechain::handle_set` does not even check the plugin exists.
  Disjoint engine state on one FIFO control thread: they commute. A state
  load can change a plugin's latency; the PDC republish after it is
  independent of the route tables.
* **App.** `plugin_state` writes `state_cache`, slot `params`, slot
  `bypassed`; the routing domains read `r.registry` track/bus ids and the
  file's plugin ids, and write `r.aux` / `r.sidechain`. Disjoint.
* **Echoes.** A load echoes nothing on success (`PluginStateSaved` only
  follows `SavePluginState`); `AuxSendChanged` / `SidechainRouteChanged`
  update mirrors `plugin_state` does not touch.

Guard: `reconcile_order::a_diff_undo_restores_plugin_state_before_routing`
(fails on the old order, where `SetAuxSend` preceded `LoadPluginState`).

### Proof 2: the diff path's resort moves from after `Tail` to before `Routing`

`entity_order` runs `resort_tracks`, `resort_busses`, `rebuild_output`,
`refresh_track_count`. Between the old and new positions run `Routing`,
`Clips`, `Content` and `Tail`. Before, those saw the registry in its
pre-undo vector order (with the target `.order` values already written
by `apply_track` / `apply_bus`); now they see it sorted. Every reader of
`registry.tracks` / `registry.busses` in those domains and what they
call:

* `routing::source_exists` and the dest-bus check — `iter().any(id)`.
* `clips::VocalAudioClips` — collects a `HashSet` of vocal track ids.
* `restore_derived_clips` — `drum_track_ids`, a `HashSet` (legacy branch
  only).
* `restore_track_groups` — bumps `next_track_id`; no iteration.
* `apply_freeze_restore` / `reconcile_freeze_statuses` /
  `attach_freeze_cache` (`update/freeze.rs`) — `find(id)` / `any(id)`.
* `restore_external_instruments`, `restore_automation_lanes` — iterate
  the *target file*, not the registry.
* references, pool, quantize, performance, take groups — do not read the
  registry.

None depends on vector order, and none writes it (no domain after
`Entities` pushes, removes or reorders a track or bus), so sorting
earlier can be neither undone nor observed. `rebuild_output` reads bus
names, which `busses` has already written; `output_choices` and
`compose.track_count` are read only by views. No engine command is
involved. Guards: `a_diff_undo_resorts_the_registry` (behaviour) and the
trace tests (position).

Full path, same domain: `finalize_plugin_chains` moves from after `Clips`
to before `Routing`, and `resort_tracks` / `refresh_track_count` /
`resort_busses` / `rebuild_output` from right after their replay loops to
the end of `Entities`. Readers in between: `replay_bus` (reads
`next_bus_order`), `migrate_old_generate_params` (keys lane generators by
track id — order-insensitive, and it stayed in `tracks`), `master`,
`track_outputs` and `plugin_state`'s full arm (iterate the file),
`Routing` and `Clips` (read the file's plugin ids, never
`plugin_mirror.index`; nothing in either calls `with_plugin_mut`). The
chain sort is stable and, since every `PluginAdded` echo is handled only
after the synchronous replay returns, it still runs before any echo can
have appended a slot — as before.

### Proof 3: the full path's `SetPluginBypass` leaves the add body

It moves into `plugin_state`, after the blob; the placeholder's
`bypassed` seed stays in the add body.

* **Why it cannot stay.** Once `LoadPluginState` leaves the add loop, a
  bypass left there would precede the blob. A slot whose plugin declares
  its own bypass parameter is bypassed by the render path pushing that
  parameter (`PluginSlot::sync_own_bypass`), edge-triggered on
  `own_bypass_sent`. If the audio thread renders between
  `SetPluginBypass` and `LoadPluginState`, it pushes `1`; the reload can
  then put the plugin's own parameter back to the blob's value, and the
  host never re-sends it — the slot shows bypassed and plays wet. With
  the blob first, the bypass target is set after the reload, as before.
  (The diff path had exactly the inverted order — `apply_plugin_bypass`
  inside the entity scalars, blobs after them — and now shares the fixed
  one.)
* **The echo still overwrites the placeholder.** Per instance the engine
  sees `AddPlugin*` (emits `PluginAdded`) before `SetPluginBypass` (emits
  `PluginBypassChanged`), on one FIFO thread, so the events keep their
  order. `track_added` / `bus_added` / `master_added` find the placeholder
  by instance id — it exists from the add body on — and
  `adopt_live_instance` overwrites only `params` / `has_gui` /
  `has_sidechain_input` / availability, never `bypassed`. The seed stays
  in the add body so the placeholder is right the moment it exists.
* **Parked params.** `apply_pending_param_overrides` runs from the echo
  handler, i.e. after the replay returns, so the overrides are parked and
  `LoadPluginState` queued before it whatever the order inside the
  replay; the per-param sends still land after the blob (comment in
  `engine_events/plugins.rs` updated).
* **Per-instance order kept.** Full path before: `Add, Load, Bypass` per
  plugin, interleaved; now every `Add` (with the other entity commands
  and the `SetTrackOutput`s), then every `Load`, then every `Bypass`.
  Different instances are independent engine state.

Guard:
`the_full_path_restores_plugin_state_after_every_entity_and_before_routing`
(fails on the old order, where `LoadPluginState` preceded
`SetTrackOutput`; it also checks the override is parked, then applied on
the echo).

### Other ordering changes

Diff path: `SetTrackOutput` moves from inside each track's scalars to
after every track, bus and the master (`track_outputs`).
`handle_set_track_output` writes one track's output atomic; the bus and
master scalars and the other tracks' scalars touch other state. Full
path: none besides the above — `SetTrackOutput` already followed the
master chain.

Every existing guard passed unchanged at each commit (fixed-point,
`undo_restore_flag`, `io::replay`, `id_allocation`, the plugin / bus /
track undo tests, the `io`, `plugins`, `mixer` and `timeline` groups).

### What this changes for A-13g–j

* **A-13h (busses, plugin instances).** The add bodies are now the
  `old = None` arms of `tracks` / `busses` / `master` (`replay_bus`,
  `replay_plugins`); a diff arm that adds what `old` lacks can call them
  per entity. A diff-added plugin then needs its blob pushed *and* its
  overrides parked (it has no params until its echo), so `plugin_state`'s
  phases must switch per instance ("fresh" vs "live"), not per origin.
  Removals must still run before `Routing` re-sends against a removed
  endpoint — §10's routing-removal pre-pass stands. `entity_order`'s
  chain sort then has to run on the diff arm too (a reorder becomes
  `MovePluginIn*`).
* **A-13i (tracks).** `tracks`' `old = None` arm clears the whole
  registry and the plugin index; a per-track add/remove on the diff arm
  must keep `plugin_mirror.index` consistent itself, or `entity_order`
  rebuilds it on every origin (cheap).
* `LiveCarry` is unchanged; nothing in `Entities` needed a carry.

## 12. Group (6), step 2: A-13g

Written against master `645d49e1` (after A-13f).

### What left the gate

`structurally_compatible` no longer looks at the id sets of entity kinds
whose domains restore them whole on every origin. One commit per check, each
with a guard in `tests/io/undo_diff_shape.rs`: a real edit on the demo
project, walked through `Message::Undo` / `Message::Redo`, where every step
must send no `ClearAll`, run all 31 domains under `Origin::UndoDiff`, and
leave `build_project_file` equal to the target snapshot's file (and the
snapshot `same_state`, notes included).

| Check dropped | Domain that restores it whole | Guard |
|---|---|---|
| arrangement markers | `Markers` (Timeline) — `ArrangementMarkers::from`, id counter recomputed | add, then delete, a marker |
| track groups | `TrackGroups` (Content) — registry rebuilt, track-id counter only rises | create a group from a selection |
| drum patterns | `DrumPatterns` (Globals) — bank replaced, default recomputed, id counter only rises | add a pattern; a hand-made empty-bank snapshot |
| legacy `drum_groups` | `DrumPatterns` (promotion) — and every snapshot writes the list empty | a group add inside a pattern; the gate unit test flipped |
| section placements | `ComposeSections` (Globals) — `load_from_project` | place a section, then delete that placement |
| section definitions | `ComposeSections` | create an unplaced section, then delete it; the GUI create (definition + placement) |

What the gate still checks: tracks (type, sub-track link), busses, the
plugin chains of both and of the master, audio and MIDI clips — what the
diff arms cannot add or remove yet (A-13h, A-13i).

### Checked against the §10 watch list

* **`drum_patterns`' `clear_on_empty`.** Now live on the diff path. For an
  empty target it empties the bank, which is what keeps the fixed point
  (`a_diff_restore_to_an_empty_drum_bank_clears_it`). No edit can produce
  such a snapshot — the last pattern refuses to delete — but the rule is
  the right one. The full path keeps the live bank for an empty file; that
  stays a disk-load rule for projects that predate drum patterns.
* **Drum-roll focus.** The diff arm leaves `managing_pattern_id` /
  `selected_group_id` / `managing_group_id` alone, so undoing a pattern
  add leaves them naming a pattern that is gone. Every reader resolves or
  compares them (`resolve_managing_pattern_id` falls back to the default
  pattern; the views match by id), so nothing acts on a stale id; the
  pattern test checks a group add after the undo lands in a live pattern.
  This was already the case for groups inside a pattern, which the gate
  never checked.
* **Placement keys in `DerivedClips` and the vocal audio-clip map.** A
  placement's derived MIDI clips and installed vocal audio clips are clips:
  deleting a placement purges them (`purge_placement_outputs`), so undoing
  that still changes the clip-id sets and still falls back. What reaches
  the diff path is a placement with no clips. The derived map is the
  target's entries (`DerivedClips` keeps every one on the diff path, as
  before), so no key can name a placement the target lacks; the vocal
  audio-clip map is rebuilt from the target's placements in
  `VocalAudioClips`, after `ComposeSections` loaded them.
* **Derived clip ids and the counter floor.** Unchanged: the floor is
  carried in `LiveCarry`, `load_from_project` resets the counter,
  `DerivedClips` / `VocalAudioClips` raise it back past the floor and every
  restored id. No derived id is allocated by a restore.
* **Readers of `r.compose` sections between `Globals` and `Clips`.** The
  only one is the `tracks` domain's `migrate_old_generate_params`, on its
  after-`ClearAll` arm only. `Entities`, `Routing` and the clip domains
  read the file, the registry and the clip mirrors.
* **Other runtime state keyed by a section.** `load_from_project` already
  cleared the vocal side-tables (lyrics, render epochs and cache),
  pronunciation and expression curves on every diff undo; the selected
  placement falls back to the first. `vocal_bulk_lyrics` and an open
  edit-section form are not reset by either path (a form naming a removed
  definition renames / resizes nothing).

### Behaviour changes

Undo / redo of any edit that adds or removes only these entities — a
marker, a track group, a drum pattern, a drum group, an unplaced section,
an empty placement, the GUI's create-section — now takes the diff path
instead of `ClearAll` + full replay. User-visible:

* **No plugin re-instantiation**, so no audible gap and no plugin state
  reload on those undos.
* **The playhead stays put.** The full path's `Transport` resets it to 0
  after `ClearAll`.
* **Transient UI survives**: selected clip and plugin, an in-flight clip
  drag or trim, the delete-track / quit confirmations (`TransientUi`), and
  the drum-roll focus (`DrumPatterns`) are kept, not reset.
* **No loading window.** The full path sets `io.loading` until
  `AllCleared`, during which control-API mutations are refused; the diff
  path is synchronous.
* **Full-path-only steps no longer run on these undos**: the chord trim
  (the target is a snapshot of already-trimmed live state), the
  missing-plugin warning dismiss, the take-lane peak-cache drop, the
  frozen-track cache re-decode, the drop of derived-map entries whose echo
  is pending. Each is the diff path's existing rule, the same as for a
  scalar undo.

### Found, not fixed (pre-existing)

* **Group macro solo / mute are not re-derived by any restore.** The group
  handlers push each member's *effective* solo / mute
  (`SetTrackSolo` / `SetTrackMute`); the entity domains restore each
  track's own flag, and on the diff path only when it changed. So a diff
  undo of a macro toggle already left the engine's effective flags stale;
  before A-13g a later structural undo (e.g. of the group's creation)
  happened to reset them via `ClearAll`, now it does not. The full path
  has the mirror image: it sends the own flag, ignoring a restored macro
  solo. Fix belongs in `TrackGroups` (re-send effective flags for every
  member whose effective value differs) — a follow-up, not a gate issue.
* **Marker ids are re-issued after an undo.** `ArrangementMarkers::from`
  recomputes `next_id = max + 1`, so undoing a marker add frees its id and
  the next add reuses it (the redo stack is cleared by that add, so no
  snapshot collides). A stale `selected_marker_id` then highlights the new
  marker. Same on both paths.

### What this changes for A-13h–j

* The gate is now exactly the entity / clip shape. A-13h drops the bus and
  plugin checks (bus / plugin add-remove arms); A-13i the track and clip
  checks; A-13j deletes the function. Nothing app-side is left for them to
  worry about in the gate.
* `transient_ui` and `drum_patterns`' focus rule say "the diff path never
  removes the entities they name". That still holds for tracks, clips and
  plugins until A-13h/i; once those are removable on the diff path, the
  transient UI (selected clip / plugin, drags, delete-track confirm) needs
  a per-entity prune on the diff arm instead of the after-`ClearAll`
  reset.
* A structural undo is still a full replay; the playhead / transient-UI /
  loading-window differences above are what users will notice change per
  entity kind as A-13h/i land.

## 13. Group (6), step 3: A-13h

Written against master `91ec9867` (after A-13g, D-7c).

### What left the gate

| Check dropped | Commit | Guard (`tests/io/undo_diff_shape.rs`) |
|---|---|---|
| bus id set | 2 | `adding_and_removing_a_bus_with_routing_undoes_through_the_diff_path` (GUI add, return role, send, key route keyed off the bus, delete — every undo/redo pinned or settled); `a_bus_restore_survives_the_previous_restores_late_echoes` |
| plugin chains of tracks, busses and the master (ids, order, `.clap` identity) | 3 | `adding_removing_and_reordering_a_{track,bus,master}_plugin_undoes_through_the_diff_path` (add two, key one, set its param and bypass, reorder, remove); `undoing_a_plugin_add_drops_its_selection`; `a_plugin_restore_survives_the_previous_restores_late_echoes`; `a_re_added_plugins_params_follow_a_second_restore_before_its_echo` (commit 4) |

Every guard asserts, after each step, no `ClearAll`, all 33 domains under
`UndoDiff`, `build_project_file` equal to the target and the snapshot
`same_state`; then plays the engine's echoes back and asserts the same
again, plus that no echo is still owed (`RestoreEchoes`). The structural
steps pin the exact command list. What the gate still checks: the track
set (type, sub-track link) and the audio / MIDI clip sets (A-13i).

### Domains

`Stage` is now `Globals, Timeline, Removals, Entities, Routing, Clips,
Content, Tail`; 33 domains. New code: `reconcile/removals.rs`.

| Stage | Domain | Body by origin |
|---|---|---|
| Removals (1st) | `routing_removals` | after `ClearAll`: nothing. Diff: `RemoveAuxSend` for every send `old` has and `new` lacks; `ClearSidechainRoute` for every route of `old` that is not kept (`kept_route_plugins`: its plugin is kept and `new` still keys it). The removal half of `Sends` / `SidechainRoutes`, moved. |
| Removals (2nd) | `entity_removals` | after `ClearAll`: nothing. Diff, in `old`'s chain order: every instance `new` does not keep → `RemovePlugin` / `RemovePluginFromBus` / `RemovePluginFromMaster` (skipped for a bus that goes too — `RemoveBus` drops its chain, with no per-plugin echo); then every bus `new` lacks → `RemoveBus`. Each pruned app-side at once (below). |
| Entities | `tracks` | diff arm also appends each plugin of a kept track that `old` did not keep (`replay_plugins`, as a load). |
| Entities | `busses` | diff arm adds a bus `old` lacks with `replay_bus` (the load body, keeping the file's `.order`), and appends new plugins to a kept bus. |
| Entities | `master` | diff arm appends new plugins. |
| Entities | `plugin_state` | fresh vs live **per instance** (below). |
| Entities (last) | `entity_order` | diff arm moves every chain into `new`'s order (`MovePlugin` / `MovePluginInBus` / `MovePluginInMaster`, `order_chain`); the plugin side-index is rebuilt on every origin. |
| Routing | `sends`, `sidechain_routes` | diff arms only upsert now; `sidechain_routes` compares only the kept routes against `new`, so a route onto a re-added instance is set again. |

**Kept, fresh, removed.** `entities::kept_plugins(old, new)`: an instance
id in both files, on the same chain (`PluginLocator`), with the same
`clap_plugin_id` and `clap_file_path`. Every other instance of `new` is
*fresh*; every other instance of `old` is removed. After a `ClearAll` the
kept set is empty, so every instance is fresh — the full path is the
special case of the rule, not a separate branch. An id whose identity
changed (undo of a relocate, `update::plugin_replace`) or whose chain
changed is removed and re-added under the same id, which is what the full
replay did to it.

### Removal ordering: a stage before `Entities`

§10 left two options: a `RoutingRemovals` pre-pass, or entity removal in a
late stage. Chosen: a `Removals` stage before `Entities`, holding both the
edge removals and the entity removals, edges first. Why not late:

1. **Chain order.** With removals first, a live chain at `EntityOrder` is
   the kept slots in their old order followed by the appended fresh ones —
   exactly the engine's chain, which appends an add. With a late removal,
   every move index would have to count slots that are about to go.
2. **Same-id re-add.** The engine refuses `AddPlugin*` for an id that is
   still live (D-1). An identity or chain change under one id needs the
   removal before the add, and a plugin moving from a bus to a track
   would otherwise be added by `tracks` before `busses` removes it.
3. **Edges before endpoints, everywhere.** `routing_removals` runs before
   any entity goes, so no mirror (or engine table) ever names a removed
   bus or plugin, and A-13e's removals-first rule (a replacement send is
   cycle-checked against a graph without the edge it replaces) now spans
   the whole restore instead of the `Routing` stage.

The engine would tolerate the other order: `handle_remove_bus` leaves the
bus's sends in the table (the live delete removes them on the
`BusRemoved` echo) and a send onto a missing bus is skipped at render
time; `RemovePlugin` drops a track plugin's key route itself. So the order
is about the mirror and about (1) and (2), not about engine safety.

**What moved (diff path):** `RemoveAuxSend` / `ClearSidechainRoute` from
after the entity scalars and plugin state to before every entity command.
`handle_remove_aux_send` and `sidechain::handle_clear` touch only
`state.aux_sends` / `state.sidechain_routes` (§10, §11), which no entity
scalar, bypass, state load or param handler reads; the echoes update
`r.aux` / `r.sidechain`, which the entity domains do not write. Guard:
`reconcile_order::a_diff_undo_removes_edges_before_any_entity_command`
(fails on the A-13g order).

**Bus removal and track outputs.** `RemoveBus` goes out before
`TrackOutputs`. `handle_remove_bus` itself moves every track routed to the
bus onto the master; `TrackOutputs` then sends `SetTrackOutput(Master)`
for those tracks (their output differs from `old`'s), a no-op re-assertion
that keeps one rule for the domain. `tracks` wrote the mirror's output
from `new` already.

### Fresh vs live in `plugin_state`

| Phase | Fresh instance | Live (kept) instance |
|---|---|---|
| blob | pushed if the target has one; cached | pushed only when the cache moved on (`Arc::ptr_eq`, FU-A2b) |
| bypass | `SetPluginBypass` when bypassed (placeholder already seeded) | sent when the slot's bypass changed, matched by id |
| params | parked in `pending_plugin_param_overrides` for the `PluginAdded` echo | driven to the target (STATE-03 / FU-A2b); a live instance still awaiting its echo (added by a previous restore, or a missing `.clap`) has its parked values replaced by the target's |

The last cell is a fix found while testing (commit 4): undo a plugin
removal (the instance is re-added, its values parked), then undo the
param edit before it before the echo lands — the second restore saw the
instance as live, drove nothing (no param list yet) and left the first
restore's values parked, which the echo then applied and a save wrote.

The fresh rules are the full path's, per instance. The bypass-after-blob
order (§11 proof 3) holds per instance on both.

### Chain order on the diff path

`order_chain(slots, target, kept, send_move)`: two left-to-right passes,
first the kept slots into their target relative order, then each fresh
slot into its target position. Each move is mirrored at once and named by
the **engine** index the slot ends at (`plugin_chain::engine_slot_index`
over the moved chain — a missing plugin keeps its place in the app's
chain but not the engine's; moving one sends nothing).

Kept first because they are known to exist. A fresh slot is `Available`
until its echo says otherwise, so a fresh plugin that turns out missing is
counted, and pass 2 can then place a *later* fresh plugin one engine slot
off. That needs two plugins re-added by one restore, one of them missing
and out of append order; the recovery path re-positions the missing one if
it ever loads. Recorded, not fixed.

### Echoes the restore owes: `io.restore_echoes`

Live edits mirror structural changes on the engine's echo; the diff
restore mirrors them itself, synchronously. Its echoes land after it
returns — and after the *next* restore when an undo and a redo run before
the event pump (a held Ctrl+Z, a control client's burst). Undo and redo
reuse ids by design, so a late echo can name what the next restore put
back. Without a guard:

* `BusRemoved` for a bus the redo re-added deletes it (and its chain,
  sends, scalars); `PluginRemoved` drops the re-added slot and its parked
  params, and the `PluginAdded` after it pushes a bare slot at the end.
* `*PluginMoved` is an absolute "move X to i": replaying one restore's
  moves on the chain a later restore left does not in general give that
  chain (found by brute force over 3-slot histories; the guard uses the
  failing case: two reorders undone back to back).
* The `*Added` echo of an instance (or a bus) a later restore removed
  pushes a phantom.

`RestoreEchoes` (`state/project_io.rs`) counts the removals and moves a
restore sent. The engine answers every `RemoveBus` / `RemovePlugin*` /
`MovePlugin*` (an unknown id too), on one FIFO thread, so each count is
settled exactly once. Handlers: a matching removal / move echo is
swallowed; an add echo for an instance or bus whose removal is still owed
is ignored (FIFO puts it before that removal, which the restore already
mirrored); `BusPluginAdded` for a bus whose removal is owed too. Both the
plugin halves and the bus half were checked to fail their guard with the
ledger disabled.

The full path never needed this: `AllCleared` is delivered after every
earlier echo, so a `ClearAll` serialised them.

### Per-entity prune

What `EntityRemovals` does to the mirror for a removed instance, as the
`*_removed` echo handlers would: the slot, its cached blob, its parked
params, its side-index entry, the mixer's `selected_plugin`, a key route
onto it. For a removed bus: the registry entry, `selected_bus`, key routes
keyed off it (its sends are `routing_removals`'). The rest needs nothing:

* **Open plugin editors** — `editor_open` lives on the slot; the engine
  drops the instance with its window.
* **Missing-plugin warning** — derived from the chains
  (`missing_plugin_slots`), so a removed missing slot leaves it; the
  modal's own open / dismissed flags are session state.
* **Output-destination picker, plugin side-index** — rebuilt by
  `EntityOrder` on every origin.
* **Transient UI (`TransientUi`), drum-roll focus** — name clips, tracks,
  confirmations and drum patterns, none of which this slice removes. They
  need a per-entity prune with A-13i (tracks, clips).

### Behaviour changes

Undo / redo of an edit that adds, removes, reorders or relocates a plugin
(track, bus or master chain) or adds / removes a bus — including the
sends and key routes that go with it — now takes the diff path.
User-visible:

* **Only the affected instance is instantiated or dropped.** Every other
  plugin keeps running: no audible gap across the whole mix, no state
  reload, other plugins' editor windows stay open, their parameter
  automation and meters are not interrupted. A reorder is `MovePlugin*`:
  nothing is re-instantiated.
* **The playhead stays put**; **transient UI survives** (selected clip,
  in-flight drag or trim, confirmations, drum-roll focus); **no loading
  window** (control-API mutations are not refused mid-undo) — as A-13g
  listed for its entities.
* **A removed plugin or bus is deselected** in the mixer (the full path
  reset every selection).
* **A re-added missing plugin** fails to load again and raises the
  missing-plugin warning unless the user has dismissed it for this
  project. The full undo dismissed it (`MissingPlugins`' `UndoFull` arm);
  the diff arm leaves the warning state alone, as for any diff undo.
* **Full-path-only steps no longer run on these undos** (chord trim,
  missing-plugin dismiss, take-lane peak-cache drop, frozen-track
  re-decode, dropping derived-map entries whose echo is pending) — A-13g's
  list.
* **Engine traffic on the diff path's routing removals** moved ahead of
  the entity commands (not observable).
* **Group macro solo / mute (FU-A13a).** Not made worse: busses and
  plugins are not group members. What changes is the same thing A-13g
  noted — a plugin / bus undo no longer happens to reset the engine's
  effective flags through `ClearAll`.

### Found, not fixed

* **Undo before the echo of a live plugin / bus delete** (the STATE-10
  shape, which fixed clips and tracks). `RemovePluginFromTrack` /
  `RemovePluginFromBus` / `RemovePluginFromMaster` / `RemoveBus` still
  mirror on the echo; an undo in between snapshots a mirror that still
  holds the entity, restores to an equal shape (a no-op) and the echo then
  deletes it. Pre-existing (the gate saw equal shapes before too). The fix
  is STATE-10's — mirror the delete at once — and the echo it then owes is
  exactly what `RestoreEchoes` records; worth a follow-up.
* **`order_chain` with two fresh plugins, one missing** — see above.

### What this changes for A-13i / A-13j

* **A-13i (tracks, clips).** Track removal belongs in `entity_removals`,
  after the edges and plugins that name it (`kept_plugins` already
  treats a plugin whose track changed as removed + re-added); track adds
  in `tracks`' diff arm via `replay_track`. `RestoreEchoes` needs a
  `TrackRemoved` count (and `ClipDeleted` for clip removals). Sub-tracks
  are created by `ensure_subtracks` on a multi-output instrument's
  `PluginAdded` echo: a diff restore that re-adds such an instrument
  together with its sub-tracks must add the sub-tracks itself before the
  echo, keyed by (parent, port), so the echo finds them and adds none.
  The per-entity prune grows: selected track and clip, clip drags, the
  delete-track confirm, the drum-roll focus, the MIDI editor, pending
  control tracks. External instruments / freeze / lanes of an added
  track need their `after_clear_all` behaviour per track (unchanged from
  §10).
* **A-13j (delete the fallback).** The kept / fresh rule already makes
  the full path a special case (empty kept set); `EntityRemovals` and
  `RoutingRemovals` are no-ops after `ClearAll`. With `Origin::UndoFull`
  gone, `MissingPlugins`' dismiss-on-undo rule goes with it — decide
  whether a diff re-add of a missing plugin should re-raise the warning.

## 14. Group (6), step 4: A-13i

Written against master `815af005` (after A-13h).

### What left the gate

| Check dropped | Commit | Guards (`tests/io/undo_diff_shape.rs`) |
|---|---|---|
| track set (id, type, sub-track link) | 1 | `adding_and_removing_{an_audio,a_vocal}_track_…`, `…_an_instrument_track_with_plugins_and_sends_…`, `a_sub_track_producing_instrument_…`, `…_an_external_instrument_track_…`, `a_re_added_frozen_track_gets_its_cache_attached`, `a_track_type_change_is_a_remove_and_an_add`, `a_re_added_group_member_gets_its_effective_mute`, `a_re_added_tracks_automation_lane_is_sent_again`, `undoing_a_track_add_drops_its_selection_and_never_reissues_its_id`, `a_track_restore_survives_the_previous_restores_late_echoes`, `undoing_a_track_delete_before_its_echo_keeps_the_track` |
| audio and MIDI clip sets (id, WAV, length) | 2 | `deleting_an_audio_clip_…`, `splitting_an_audio_clip_…`, `adding_and_removing_midi_clips_…`, `deleting_a_track_with_clips_…`, `a_clip_whose_wav_changed_is_deleted_and_reloaded`, `a_clip_restore_survives_the_previous_restores_late_echoes`, `undoing_a_clip_delete_before_its_echo_keeps_the_clip`, `a_midi_note_restore_keeps_the_clips_lyrics_through_its_echoes` |

Every guard asserts, after each step, no `ClearAll`, all 34 domains under
`UndoDiff`, `build_project_file` equal to the target and the snapshot
`same_state`; then plays the engine's echoes back (the harness now also
answers track adds and removals — a parent's `RemoveTrack` for each
sub-track the engine still holds under it — clip loads, deletes and MIDI
deletes) and asserts the same again with nothing owed. The structural
steps pin the command list. The track- and clip-ledger halves were each
checked to fail their late-echo guards with the ledger disabled.

`structurally_compatible` now checks nothing. It stays, always `true`,
for A-13j to delete together with the undo's `ClearAll` fallback. The
tests that pinned the full undo path used to force it with an extra track;
they now call `test_begin_full_restore_from_snapshot`
(`Resonance::restore_from_snapshot(snapshot, allow_diff = false)`), the
only way left to reach `Origin::UndoFull`.

### Kept, fresh, removed — tracks and clips

The A-13h rule, extended:

* **`entities::kept_tracks(old, new)`** — the id is in both files with the
  same `track_type` and `sub_track` link, and a sub-track's parent is kept
  (the engine's `RemoveTrack` drops a parent's sub-tracks). Every other
  track of `new` is *fresh*, every other track of `old` is removed. A type
  change is a remove + add under the same id, which is what the full
  replay did to it. Empty after a `ClearAll`.
* **`kept_plugins`** additionally requires a track chain's track to be
  kept: `RemoveTrack` drops the chain, so a re-added track's plugins are
  fresh even under unchanged ids.
* **`clips::kept_audio_clips` / `kept_midi_clips`** — in both files, on a
  kept track (in `old`), and for audio the same `audio_file` and
  `total_frames`. A clip whose WAV or length changed is deleted and
  reloaded under its id.

### Where each piece runs

| Stage | Domain | A-13i change |
|---|---|---|
| Removals (1st) | `routing_removals` | `kept_route_plugins` also drops a route keyed off a removed track (the engine's `RemoveTrack` drops it with the track), so `SidechainRoutes` sets it again onto a re-added one. Sends are left alone: `RemoveTrack` does not prune the engine's send table, and a send whose source goes is absent from `new` anyway. |
| Removals (2nd, **new**) | `clip_removals` | Diff: `DeleteClip` / `DeleteMidiClip` for every clip of `old` not kept; mirrored at once, echo owed, transient UI pruned. |
| Removals (3rd) | `entity_removals` | After the plugins (a plugin on a removed track sends nothing — the track takes it), every removed track: sub-tracks first, then the rest, each by its own `RemoveTrack`, then the busses. |
| Entities | `tracks` | Diff arm: kept tracks as before; then every fresh track through `replay_track` (the load body, now taking the order), parents before sub-tracks, keeping the saved `.order`; `next_track_id` / `next_track_order` bumped past each. |
| Entities | `track_outputs`, `plugin_state` | Per fresh track: `SetTrackOutput` only for a bus route; its plugins fresh (blob, bypass after the add, params parked). |
| Clips | `audio_clips`, `midi_clips` | Diff arm: kept clips as before; every fresh clip through `load_audio_clip` / `load_midi_clip`; then the mirror sorted into `new`'s order (the serializer writes it in vector order — a clip re-added at the end broke the fixed point). A MIDI reload's delete echo is owed (below). |
| Content | `track_groups` | Group-macro sync: a fresh track's engine flags are the own flags `replay_track` just sent, so the effective mute / solo goes out only where it differs (FU-A13a for a re-added member). |
| Tail | `external_instruments` | A fresh track gets the after-`ClearAll` rule on its own: its map entry is rebuilt (online) and it gets no empty `SetTrackDeviceParams` when no device is selected. A removed track is in the stale set: `ClearExternalInstrument` + empty params, since the engine's `RemoveTrack` keeps its external config. |
| Tail | `freeze` | A fresh track is not in the "attached" set, so a restored freeze decodes and attaches its cache (`SetTrackFrozenSource`); a removed frozen track is detached and its cache deleted, as the live delete does. |
| Tail | `automation_lanes` | Unchanged: the engine keeps lanes keyed by target across `RemoveTrack`, and the domain diffs against the mirror, which the live delete had cleared — so a re-added track's lane is sent again (guarded). |

`removals.rs` is now three domains: edges, clips, entities — the
"removals before adds, edges before endpoints" rule of §13, with clips as
the edges of a track.

**Why clips go before tracks, in their own domain.** The design sketch
(§13) put clip deletion in the clip domains' diff arms. They run after
`Entities`, i.e. after `RemoveTrack`, which drops a track's audio clips
*without an echo* and keeps its MIDI clips. A `DeleteClip` after it is
parked by the engine's load-deferral queue (`defer_clip_command`) for a
clip that never lands, times out with an error and never echoes — an
owed echo that never settles. Deleting every clip first, explicitly,
gives one command and one echo per clip whatever happens to its track.
(Since FU-A13e/f a `DeleteClip` is never parked for its load and always
echoes; the order stays, since `RemoveTrack` still drops clips silently.)
Adds stay in the clip domains (after the tracks they sit on exist).

### Sub-tracks

`ensure_subtracks` creates a multi-output instrument's sub-tracks on its
`PluginAdded` echo, allocating fresh ids. A restore that re-adds such an
instrument adds the sub-tracks the target names itself, under their saved
ids (`CreateSubTrack`, no echo), in the same synchronous pass as the
`AddPlugin` — so when the echo runs, every (parent, port) is taken and it
adds none. Removal: a sub-track `new` lacks while its parent stays (undo
of the plugin add) is removed by its own `RemoveTrack`; a removed parent's
sub-tracks are removed first, each by its own `RemoveTrack`, so the parent's
`RemoveTrack` answers only for itself and every sub-track's plugin chain
is dropped (the engine's parent `RemoveTrack` drops the sub-tracks but not
their chains).

### Echoes the restore owes

`RestoreEchoes` gains `TrackRemoved`, `ClipDeleted` and `MidiClipDeleted`
counts. Handlers (`engine_events::{tracks, clips, midi}`):

* a matching removal echo is swallowed (`removed_echo`, `deleted_echo`,
  `clip_deleted_echo` — dispatch now calls these, the mirror functions
  stay for the live paths);
* every other echo naming an entity whose removal is owed is ignored —
  FIFO puts it before the removal, so it describes the instance already
  gone: `*TrackAdded`, `PluginAdded` on that track (which would otherwise
  run `ensure_subtracks` on a missing parent), `ClipImported`,
  `MidiClipCreated`, and the clip placement echoes (`ClipMoved` /
  `ClipTrimmed` / fade / gain, MIDI moved / trimmed). An audio clip's
  `ClipImported` comes from the load worker, not the command thread, but a
  `DeleteClip` of a loading clip cancels that load (FU-A13e), so a load
  echo never follows its delete's.

**Live deletes owe their echo too.** STATE-10 made the GUI track and clip
deletes mirror at once, and their echoes found nothing left to drop.
Before A-13i the undo of such a delete was a `ClearAll`, and `AllCleared`
arrived after the delete's echo; now the diff restore re-adds the entity
under the same id at once, and the late echo would remove it again. So
every live path that mirrors a deletion at once now owes its echo:
`ConfirmRemoveTrack` (the track and each sub-track), the track removal's
own MIDI clip deletes, the GUI clip delete, the arrange-span delete, the
placement purge, `forget_track`, `install_derived_midi_clip` and the vocal
MIDI install (`engine_events::{clips, midi}::send_mirrored_delete`). The
engine echoes every audio and MIDI delete — since FU-A13e/f an audio
delete of an id it never loaded (missing media, a cancelled load) too —
so both are owed whether or not the mirror held the clip. Paths that mirror on the echo (the GUI MIDI-clip delete, the
vocal-audio re-install) owe nothing. Guards:
`undoing_a_{track,clip}_delete_before_its_echo_keeps_the_{track,clip}`.

### Per-entity prune

The after-`ClearAll` `TransientUi` reset is replaced, for removed tracks
and clips, by a prune in the removal domains:

* **Track** (`removals::prune_track`): the registry entry, the track
  selection (`deselect_track`), its context menu, preset-save prompt and
  membership drag, the delete-track confirmation, the bounce dialog, the
  mixer's expanded sub-track parents, automation / take-lane expansion,
  the Compose focus (`expanded_track_id`, and `selected_lane` —
  instrument or drum-roll — back to Chords), a control client's pending
  track. Freeze, external config, lanes, group membership, sends and
  routes, compose tables are reconciled by their domains.
* **Clip** (`removals::prune_clip`): the mirror entry, the selected audio /
  MIDI clip, a clip drag, trim, fade or gain gesture, MIDI clip drag or
  trim, the open MIDI editor and pitch editor. Pool usage, lyrics and the
  derived / vocal-audio maps are rebuilt by their domains.

### Checked, no change needed

* **D-4 track ids.** The file's ids are re-added; the engine accepts them
  (the removal ran first, or the id was never live) and `next_track_id`
  only rises (guarded: an add after undoing an add gets a new id).
* **Clip WAVs (V6).** `snapshot_for_undo` sends `PersistClipWavs` before
  the edit that removes a clip, so a re-add's `clip_<id>.wav` exists; the
  undo's own snapshot persists the current clips before the restore's
  commands (`undo_clip_audio_persist` rewritten for the diff path).
* **Derived clips, vocal audio map, lyrics.** Restored whole from the
  target after the clips (`DerivedClips`, `VocalAudioClips`,
  `ClipLyrics`), so a removed or re-added derived clip needs nothing more.
* **Take groups.** `TakeGroups` re-sends every take clip load on every
  origin ("the cache skips the read, never the load"), so a re-added
  track's take clips — dropped by `RemoveTrack` — come back.
* **Automation lanes.** See the table.

### Behaviour changes

* **Every undo / redo now takes the diff path.** Adding, deleting,
  splitting, recording or generating a track or clip no longer
  re-instantiates every plugin on undo: only the tracks and clips that
  differ are touched. The playhead stays, transient UI survives (except
  what names a removed entity), and there is no loading window — a
  control-API mutation is no longer refused as `busy` during an undo
  (`control_mutation_gate_loading` now forces the full path to test the
  gate, which disk loads still use).
* **Full-path-only steps no longer run on any undo**: chord trim,
  missing-plugin dismiss, the take-lane peak-cache drop, the drop of
  derived-map entries whose echo is pending (A-13g's list), and the
  re-decode of every frozen track (a kept frozen track keeps its source; a
  fresh one is decoded).
* **A MIDI note undo keeps the clip's lyrics.** A kept MIDI clip whose
  notes changed is reloaded (`DeleteMidiClip` + `LoadMidiClipDirect`);
  the delete's echo used to drop the mirror's clip *and its lyric
  side-table entry*, and the load's echo brought the clip back without its
  lyrics — a vocal clip lost its lyrics on a note undo once the echoes
  landed, and the next save wrote them lost. Pre-existing; found by
  `a_midi_note_restore_keeps_the_clips_lyrics_through_its_echoes`.
* **Test harnesses that echo MIDI loads must echo MIDI deletes too**
  (`undo_snapshot_fixed_point`, `freeze_stale_on_content`): a re-derived
  slot is a delete + load under one id, and the load echo is ignored while
  the delete's is owed — as the real engine, which echoes both in order,
  never leaves it.

### Found, not fixed

* **Audio clip load race in the engine** — fixed (FU-A13e, below). Loads are asynchronous; a
  `DeleteClip` of a still-loading clip is parked until it lands. A load,
  delete and re-load of one id within one load's latency (a held Ctrl+Z
  over a clip delete) can let the second load's worker see the first
  clip still published and drop itself as a duplicate, after which the
  parked delete removes the only copy: the engine ends without the clip
  the app shows. Fix is engine-side (a delete should cancel an in-flight
  load of that id rather than wait for it).
* **An owed audio delete that never echoes** — fixed (FU-A13f, below). A clip whose WAV never
  loaded (missing media) is mirrored; its `DeleteClip` parks, times out
  and never echoes, so its ledger entry stays and later echoes naming the
  id are ignored (a re-added clip's `ClipImported` would not set its
  peaks). Rare; the entry is harmless otherwise.
* **Sub-track plugin chains leak on a live parent delete.** The engine's
  `RemoveTrack` of a parent drops its sub-tracks but not their plugin
  instances. The restore avoids it (sub-tracks first); the live delete
  does not.
* **The GUI MIDI-clip delete mirrors on the echo** (FU-A13c's shape): an
  undo before that echo is a no-op and the echo then deletes the clip.
  Mirroring it at once plus `send_mirrored_delete` would fix it, as for
  audio clips.
* **Late scalar echoes of a removed-and-re-added track** (e.g.
  `PlaybackSourceChanged`) are not filtered by the ledger, only add and
  clip-placement echoes are. They carry the old instance's values, which
  a delete + re-add from adjacent snapshots normally repeats.

### What this leaves for A-13j

* `structurally_compatible` (always `true`), the `allow_diff` switch and
  `test_begin_full_restore_from_snapshot`, the undo's `ClearAll` branch in
  `restore_from_snapshot`, `io.restoring_undo`, `Origin::UndoFull` and
  every `after_clear_all()` / `UndoFull` arm that only it reached. The
  tests that force the full path (`undo_snapshot_fixed_point`'s slow
  paths, `undo_restore_flag`, `freeze_persist::undo_reattach`,
  `freeze_stale_on_content`, `reconcile_order`'s full-undo trace,
  `clip_fade_gain_handlers`, `reference_echo_races`,
  `control_mutation_gate_loading`) go or move to the disk-load path.
* The kept / fresh rules make the after-`ClearAll` behaviour the empty-kept
  special case in every domain, so collapsing the origins leaves
  `DiskLoad` (full, `old = None`) and `Undo` (diff). The remaining
  per-origin differences are real rules: `MissingPlugins` (dismiss vs
  re-raise — product decision, §13), `TransientUi` / playhead reset and
  chord trim on a disk load, `References`' monitor source, `DerivedClips`'
  keep-rule, `TakeGroups`' peak-cache drop, `drum_patterns`'
  `clear_on_empty`.
* `LiveCarry::project_path` can go once nothing `take()`s
  `io.project_path` (FU-A7a).

### FU-A13e / FU-A13f: a delete cancels the load

Engine side (`resonance-audio/src/engine/clip_loads.rs`,
`clips::handle_delete_clip`). Every `LoadClipFromWav` /
`LoadTakeClipFromWav` is issued a ticket for its clip id at submit,
superseding any earlier one; the worker publishes only if its ticket is
still the id's current one, checked under the same `ctx.clips.write()` as
the FU-D7c `clear_generation` fence and the take-park delivery.
`DeleteClip` is no longer parked for its load: under that lock it
withdraws the id's ticket and removes the clip, drops edits parked for the
id (they were aimed at the instance being deleted), and echoes
`ClipDeleted` — exactly once, whether the clip had landed, was still
loading, failed to load or was never heard of. The one delete that still
waits is one of the tail a parked `SplitClip` will create; it runs once no
such split is parked (it ran, expired, or its parent was deleted).

Interleavings the old deferral got wrong (load A, delete, load B, one id;
`tests/engine/clip_delete_cancels_load.rs`, the import pool held so each
test runs the worker jobs in the order it pins):

| Order | Before | Now |
|---|---|---|
| B lands, loop replays the delete, A lands | A's clip survives (wrong audio), `ClipImported` twice | B only |
| A lands, loop replays the delete, B lands | right clip, but A's `ClipImported` echoes | B only, one `ClipImported` |
| A lands, B lands (duplicate, dropped), delete replayed | no clip | B only |
| A's trim parked before the delete | trim applied to A, extra echoes | trim dropped |
| A's WAV missing, then delete | parks 10 s, errors, never echoes | `ClipDeleted` at once |

App side: `send_mirrored_delete` now owes the echo unconditionally — an
unowed echo of a delete of a not-yet-mirrored clip would remove a clip an
undo put back under the id before it landed
(`tests/timeline/clip_delete_echo_owed.rs`). An unowed `ClipDeleted` of an
unknown id (the vocal-audio re-install's delete of a cancelled load) is a
no-op on the mirror.

## 15. A-13j: the fallback deleted

Written against master `73bd9ad1` (after A-13i and B-1).

Since A-13i every undo already took the diff path; the full-replay undo
was reachable only through a test hook. A-13j deletes it. No user-visible
behaviour changes beyond what §14 already recorded.

### What went

| Deleted | Where |
|---|---|
| `structurally_compatible` (always `true`), `id_set_eq` (only it used it) | `replay_diff.rs` |
| `try_diff_replay` — its body is now `begin_restore_from_snapshot`'s | `replay_diff.rs` |
| `restore_from_snapshot(snapshot, allow_diff)` and its `ClearAll` branch (`io.loading` / `pending_load` / flag set by an undo) | `undo/snapshot.rs` |
| `test_begin_full_restore_from_snapshot`, `test_restoring_undo` | `test_support/project.rs` |
| `io.restoring_undo`, its read in `all_cleared`, and FU-A7a's two clears (`ProjectLoaded(Ok)`, `begin_instantiate`) | `state/project_io.rs`, `engine_events/project_io.rs`, `update/project_io/{mod,instantiate}.rs` |
| `Origin::UndoFull`; `UndoDiff` renamed `Undo` | `reconcile/mod.rs` |
| `References`' third body and `ReferenceMonitorSource` (its `Live` variant served only `UndoFull`); `restore_references` now always takes the monitor from the file | `reconcile/restored.rs`, `replay/restore.rs` |
| `MissingPlugins`' dismiss-on-undo arm | `reconcile/restored.rs` |
| `TakeGroups`' `UndoFull` arm (full `clear()` on an undo) | `reconcile/app_side.rs` |
| `LiveCarry::project_path` and `LiveCarry`'s lifetime | `reconcile/mod.rs` |
| `after_clear_all` on `apply_freeze_restore` / `reconcile_freeze_statuses` (an undo is never after a `ClearAll`) | `update/freeze.rs` |

**`LiveCarry::project_path`: verified.** Its only reader was `Freeze`'s
undo arm. The full replay `take()`s `io.project_path`, which is why it
was carried; the undo builds `ctx.project_dir` from the same
`io.project_path` clone, so `Freeze` now passes `ctx.project_dir`.
`replay_loaded_project` still clears `io.project_path` for its duration
(`all_cleared` puts it back): no domain reads it — every one resolves
against `loaded.project_dir` — but the test helpers that call the replay
directly (without `all_cleared`) would then keep the previous path, so
the line stayed rather than change what they leave behind.

### One entry point

`reconcile::reconcile_all(r, old: Option<&ProjectFile>, new, ctx)` (was
`reconcile_all_stages`) clears `io.reconcile_trace` and runs `DOMAINS`.
It `debug_assert`s that `old.is_some()` exactly when `ctx.origin` is
`Undo`. Two callers:

* **Disk load / template** — `replay_loaded_project`, from `all_cleared`
  after the load's `ClearAll`: the vocal side-table clear, `SetProjectDir`,
  then `reconcile_all(r, None, file, ctx)` under `Origin::DiskLoad`. The
  disk-load tail (patch resend, scroll reset, relink modal, job
  completion) stays in `all_cleared`, now unconditional: every
  `pending_load` is a disk load.
* **Undo / redo** — `begin_restore_from_snapshot`: `Stop`, build the live
  file, then `reconcile_all(r, Some(&current), &target.file, ctx)` under
  `Origin::Undo`, synchronously.

`replay_diff.rs` keeps only `midi_notes_equal` (used by `MidiClips` and
the snapshot equality).

### What is left per origin

Every remaining `Origin` / `after_clear_all()` branch is a real rule
between a disk load and an undo, not a leftover of the fallback:

| Domain | Disk load | Undo |
|---|---|---|
| `Transport` | playhead to 0 | playhead stays |
| `TransientUi` | selection, drags, confirms reset | kept (removal domains prune what names a removed entity) |
| `DrumPatterns` | empty file keeps the seeded bank; drum-roll focus reset | `clear_on_empty`; focus kept |
| `SectionChordTrim` | trims | no (the snapshot is already trimmed) |
| `Pool` | `ReserveAssetIds` | no (allocator monotonic in a session) |
| `TrackGroups` | every effective mute / solo sent | only what differs |
| `TakeGroups` | `clear()` (drops the peak cache) | `clear_for_snapshot()` |
| `DerivedClips` | drops entries whose clip was not loaded; scans the bundle | keeps every entry (echo may be in flight); counter floor |
| `References` | `restore_references` (monitor from the file) | `reconcile_references` (monitor untouched) |
| `ExternalInstruments` | map rebuilt, no empty `SetTrackDeviceParams` | stale cleared, offline flags kept (fresh tracks as a load) |
| `MissingPlugins` | `reset()` | nothing (below) |
| `Freeze` | `reset()` + rehydrate | `apply_freeze_restore` |

### MissingPlugins: the default taken (the user may overrule)

The full-replay undo re-added every plugin on every history step and so
dismissed the missing-plugin warning rather than re-raise it each time.
That rule went with `UndoFull`. The undo now leaves the warning alone:
it re-adds only the plugin instances the live state lacks, and a missing
one among them is refused by the engine as on a load, which re-raises the
warning **unless the user already dismissed it for this project**
(`MissingPluginsState::dismissed`, reset by the next disk load). So:
undoing the removal of a missing plugin warns once, and never again after
a dismiss. **This is a default, not a decision** — the alternative (an
undo never raises the warning) is one line in `MissingPlugins::reconcile`
(`dismiss()` under `Origin::Undo` when a re-added plugin is refused, or
unconditionally) if the user prefers it.

### Tests

Removed (they tested only the full-replay undo, which no longer exists):

* `reconcile_order::a_full_undo_runs_every_domain_in_table_order` —
  replaced by `a_structural_undo_runs_every_domain_in_table_order_without_a_clear`
  (the same track-add shape now runs the table under `Undo`, no `ClearAll`).
* `freeze_persist::a_full_replay_undo_reattaches_every_restored_freeze` —
  its rule (re-attach every freeze after a `ClearAll`) is `UndoFull`-only;
  the undo arm is `a_diff_path_undo_and_redo_reconcile_the_engine_source`,
  the disk load's rehydrate the fixture of the same module.
* `replay_diff.rs`: the eleven `structurally_compatible` / `id_set_eq`
  tests (`empty_projects_are_structurally_compatible`,
  `scalar_only_track_diff_is_compatible`,
  `added_removed_renumbered_and_retyped_tracks_are_compatible`,
  `added_plugin_is_compatible`, `plugin_reorder_is_compatible`,
  `plugin_clap_identity_change_is_compatible`, `id_set_eq_ignores_order`,
  `track_reorder_alone_is_compatible`, `audio_file_path_change_is_compatible`,
  `legacy_drum_group_id_set_change_is_compatible`,
  `bus_set_change_is_compatible`). `midi_notes_equal_field_wise` stays.
* The full-replay halves of `undo_snapshot_fixed_point::automation_lanes_restore_identically_through_both_paths`
  (a duplicate of its track-delete undo half) and of
  `a6_a_pending_echo_entry_restores_through_both_paths` (now
  `…_through_an_undo`; its drop rule is a disk-load rule, below).

Moved to a disk load (they test something a disk load still does):

* `reference_echo_races::a_full_replay_restore_cancels_an_in_flight_load`
  → `a_disk_load_cancels_an_in_flight_load` (the real `ProjectLoaded` →
  `ClearAll` → `AllCleared` round trip through the fixture's player).
* `clip_fade_gain_handlers::restore_into_full_replay_path_reapplies_fade_gain`
  → `a_disk_load_reapplies_fade_gain`.
* `control_mutation_gate_loading::a_control_edit_during_a_slow_path_undo_is_busy`
  → `a_control_edit_during_a_project_load_is_busy`, plus
  `a_control_edit_right_after_a_structural_undo_is_accepted` (no window).
* `undo_snapshot_fixed_point::derived_clips_with_a_pending_echo_survive_both_restore_paths`
  → `…_survive_an_undo_and_drop_on_a_load`: its second half replays the
  snapshot as a disk load, which drops the entry whose clip it did not
  load.
* `undo_restore_flag` (rewritten for disk load vs undo): the disk-load test
  stays (now checks the `DiskLoad` trace); `an_undo_slow_path_takes_the_undo_branches`
  → `an_undo_takes_the_undo_branches_in_place` (no `ClearAll`, `Undo`
  trace, freeze retired, no patch resend, path kept, a stray `AllCleared`
  replays nothing); FU-A7a's two supersede tests →
  `an_undo_during_a_{project_load,template_instantiate}s_clear_is_refused`
  — the race has no undo side left, so the successor pins the other order:
  an undo while a load's `ClearAll` is in flight is refused
  (`can_undo_redo_now` needs `!io.loading`) and the load replays as a disk
  load.

Converted to a structural undo (a track added after the snapshot, now
restored in place): `freeze_stale_on_content::a_full_replay_undo_keeps_the_frozen_content_baseline`
→ `a_structural_undo_keeps_the_frozen_content_baseline`, and the "slow
path" halves of `undo_snapshot_fixed_point`'s `check_both_paths` (five
fixtures), `vocal_lyric_shapes_…`, `a_lyric_normalising_restore_…`,
`freeze_states_…`, `reference_content_…` and
`a6_derived_clips_restore_…_across_a_section_resize` — each through one
helper, `restore_over_a_track_add`, asserting no `ClearAll` and the fixed
point.

### Found, not fixed

* **An undo across a landed derived-clip echo leaves an orphan entry.**
  Snapshot S is taken while a re-derived clip's `MidiClipCreated` is in
  flight (S has the map entry, not the clip); the echo lands; an undo to
  S deletes the clip (`ClipRemovals`: in `old`, not in `new`) but
  `DerivedClips` keeps the entry, since on an undo it assumes an unmirrored
  clip's echo is still coming. The full-replay undo used to drop it (§14
  listed this among the full-path-only steps). Harmless until it matters:
  the entry suspends the UPD-05 freeze check on its track until the next
  regenerate replaces it. Fix: on an undo, drop an entry whose clip is in
  `old`'s MIDI clips but not `new`'s (the restore removed it; nothing will
  echo it back). Not done here because it makes the restored map differ
  from the snapshot's, which the fixed point then has to allow for.
* **Each undo builds the live file twice**: `try_undo` / `try_redo` take
  `snapshot_for_undo()` for the other stack, then
  `begin_restore_from_snapshot` builds `current` again. Passing the first
  one's file in saves one `build_project_file` (~0.3 ms debug on the demo)
  per history step.
* `detach_and_delete_cache_in` existed for a directory other than
  `io.project_path`'s (the full replay had taken it); the undo now passes
  that same path, so it can likely fold into `detach_and_delete_cache`
  (check `test_apply_freeze_restore`'s caller first).

### Epic A "Done when" (refactor-intent.md), checked on this branch

| Item | Status | Check |
|---|---|---|
| `UndoExtras`, `pending_undo_extras`, `finalize_undo_restore` deleted | ✔ | `grep -rn "UndoExtras\|pending_undo_extras\|finalize_undo_restore" resonance-app/src --include='*.rs'` — comments only, no code |
| Both restore paths go through one `Reconcile` driver | ✔ | `grep -rn "reconcile_all(" resonance-app/src` — one definition (`reconcile/mod.rs`), two callers (`replay/mod.rs` disk load, `undo/snapshot.rs` undo); `try_diff_replay` / `structurally_compatible` / `UndoFull` have no hits |
| `classify.rs` has no catch-all arms | ✔ | `grep -nE '\(_\) => UndoAction::(Skip\|Record)' resonance-app/src/undo/classify.rs` empty; `arch-invariants::undo_classification_has_no_catch_all_arms` green |
| `Resonance` ≤ 40 fields | ✔ (40) | field lines between `pub struct Resonance {` and its `}` in `resonance-app/src/lib.rs`: 40 |
| Fixed-point test green throughout | ✔ | `cargo test -p resonance-app --test io -- undo_snapshot_fixed_point` — 18/18 |

### A-14 (delta snapshots): not worth it now

Its precondition — one diff engine, not two — holds. But the reconcile
driver diffs a `ProjectFile` into engine commands, not into a file delta,
so a delta snapshot would need a second diff (file → file patch) anyway.
And the cost it would cut is small already: plugin blobs (A9-2) and MIDI
notes (A-9) are `Arc`-shared between consecutive snapshots, so a history
entry costs the `ProjectFile` skeleton, and `snapshot_cost_probe_on_demo_project`
measures `snapshot_for_undo` at ~0.28 ms (debug, demo project with six
1 MiB blobs). Revisit only if a probe on a large project shows history
memory or snapshot time mattering; the double build above is the cheaper
win first.
