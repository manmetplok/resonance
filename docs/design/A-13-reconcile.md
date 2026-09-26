# A-13: the `Reconcile` trait, one domain order for both restore paths

Design for `refactor-intent.md` Epic A item 13 (`arch-migration-plan.md`
ARCH-01 step 3, "A-13 roadmap" in the A-7 progress note). Written against
master `d6d89413`. This document covers the trait, the driver, and the first
slice (A-13a, roadmap group 1); §7 records the second (A-13b, group 2,
written against master `c325335a`), §8 the third (A-13c, group 4, against
master `d538d5cf`; group 3 waits for D-2/D-3). Later slices move one group
at a time.

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
  than proven away.
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
