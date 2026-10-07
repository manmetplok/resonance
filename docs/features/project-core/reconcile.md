# Reconcile: restoring a project into app and engine

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/refactor/A-13-reconcile.md` (design, §15 is the
> final state), `docs/history/refactor/arch-migration-plan.md` → "A-13j
> landed", `docs/history/refactor/A-6-derived-clips.md`.

There is one piece of code that turns a `ProjectFile` into live app state and
engine commands. A disk load, a template instantiation and every undo/redo go
through it. The difference is only the starting point: after a load the
engine has just been emptied, after an undo it still holds the current
project and only the differences are sent.

## Behaviour

- **Two origins.** `Origin::DiskLoad` (`old = None`, after `ClearAll` /
  `AllCleared`; called by `replay_loaded_project`) and `Origin::Undo`
  (`old = Some(current file)`, no `ClearAll`, synchronous; called by
  `restore_from_snapshot_against`).
- **One table.** `DOMAINS` lists 36 domains, each a `Reconcile` impl, run in
  table order by `reconcile_all`. The table is grouped by `Stage`:

| Stage | Domains (in order) | Why here |
|---|---|---|
| Globals | Transport, TransientUi, ComposeSections, DrumPatterns | `SetBpm` before the tempo map; sections reset the derived map |
| Timeline | TempoEvents, ChordTrack, Markers, SectionChordTrim | tempo map before any clip; chord trim reads the meter |
| Removals (undo only) | RoutingRemovals, ClipRemovals, EntityRemovals | edges before endpoints, clips before tracks, entities before a re-add under the same id |
| Entities | Tracks, Busses, Master, TrackOutputs, PluginState, EntityOrder | outputs once every bus exists; blob → bypass → params; order last |
| Routing | Sends, SidechainRoutes | the engine rejects an edge naming an unregistered endpoint |
| Clips | AudioClips, MidiClips, ClipLyrics, DerivedClips, VocalAudioClips | clips after their tracks; derived state after clips |
| Content | References, Pool, Quantize, Performance, TrackGroups, TakeGroups | pool counts clip asset refs |
| Tail | ExternalInstruments, AutomationLanes, MidiBindings, MissingPlugins, Freeze, ClipIdGrant | lanes need device bindings; freeze fingerprints everything; the clip-id grant needs the seeded counter (load only) |

- **Diff semantics on undo.** One engine command per changed scalar, one add
  or remove per entity that differs. A plugin instance with the same id,
  chain, CLAP id and path is kept alive (`kept_plugins`); a track with the
  same id, type and sub-track link is kept (`kept_tracks`). Fresh tracks get
  the full "after ClearAll" treatment.
- **Echo bookkeeping.** A restore that removes or moves something registers
  the engine echo it owes in `io.restore_echoes` (`RestoreEchoes`), so a
  late `TrackRemoved` / `PluginRemoved` / `ClipDeleted` echo does not undo
  the restore, and a stale scalar echo for a removed track's old incarnation
  is ignored.
- **Load-only steps** (not domains): clear the vocal side-tables and send
  `SetProjectDir` before the table runs (`replay_loaded_project`); after it,
  `all_cleared` resends external-instrument patches, opens the relink modal
  for missing media and resolves a pending control load job.
- **Derived clips** (compose section → clip map) are restored from the file's
  `derived_clips`, not rebuilt; an entry whose clip is not mirrored is kept
  only if that clip's creation echo is still in flight.
- **Counters are only raised.** The clip-id counter is raised past every
  restored clip and take `clip_ref`; markers keep their live `next_id`.
  Nothing a restore does lowers an allocator (see
  [entity-ids.md](entity-ids.md)).
- **MissingPlugins default:** an undo that re-adds a missing plugin raises
  the missing-plugin warning again unless the user dismissed it for this
  project. The design doc marks this as a default the user may overrule.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Driver + table | `resonance-app/src/update/project_io/reconcile/mod.rs` | `Reconcile`, `ReconcileCtx`, `Stage`, `DOMAINS`, `reconcile_all`, `domain_order` |
| Globals / timeline | `reconcile/globals.rs`, `reconcile/app_side.rs` | `Transport`, `TransientUi`, `ComposeSections`, `DrumPatterns`, `SectionChordTrim`, `TempoEvents`, `ChordTrack`, `Markers` |
| Removals | `reconcile/removals.rs` | `RoutingRemovals`, `ClipRemovals`, `EntityRemovals` |
| Entities | `reconcile/entities.rs`, `reconcile/plugin_state.rs` | `Tracks`, `Busses`, `Master`, `TrackOutputs`, `EntityOrder`, `PluginState`, `kept_tracks`, `kept_plugins` |
| Routing / clips | `reconcile/routing.rs`, `reconcile/clips.rs` | `Sends`, `SidechainRoutes`, `AudioClips`, `MidiClips`, `VocalAudioClips` |
| Tail | `reconcile/restored.rs` | `DerivedClips`, `References`, `ExternalInstruments`, `AutomationLanes`, `MidiBindings`, `MissingPlugins`, `Freeze`, `ClipIdGrant` |
| Load entry | `resonance-app/src/update/project_io/replay/mod.rs`, `replay/restore.rs` | `replay_loaded_project`, `restore_*` helpers |
| Undo entry | `resonance-app/src/undo/snapshot.rs` | `restore_from_snapshot_against`, `restore_derived_clips`, `restore_external_instruments`, `restore_automation_lanes` |
| State | `resonance-app/src/state/project_io.rs` | `Origin`, `RestoreEchoes`, `ProjectIoState::reconcile_trace` |

Invariants a change must keep:
- A new persisted domain is a new `Reconcile` impl placed in `DOMAINS` at the
  stage its dependencies require — never inline code in one caller.
- `build_project_file(restore(snapshot)) == snapshot.file` for every edit.
- `reconcile_all` asserts `old.is_some() == origin.is_undo()`.

## Surfaces

- **GUI / Control API:** none of its own; reached through open, new,
  template, `edit.undo` / `edit.redo` (see [save-open-new.md](save-open-new.md),
  [undo-redo.md](undo-redo.md)).

## How it's tested

| What | Where | Run |
|---|---|---|
| Table order on both origins (`reconcile_trace`), command order | `resonance-app/tests/io/reconcile_order.rs` | `./scripts/run-tests.py -p resonance-app` |
| Fixed point (19 tests, derived clips included) | `resonance-app/tests/io/undo_snapshot_fixed_point.rs` | same |
| Undo across add/remove of every entity kind, no `ClearAll` | `resonance-app/tests/io/undo_diff_shape.rs` | same |
| Load vs undo take different restores | `resonance-app/tests/io/undo_restore_flag.rs` | same |
| Replay helpers | `resonance-app/tests/io/replay.rs`, `replay_diff.rs` | same |

Not covered: the real engine's response to a diff restore (tests capture
commands), and plugin re-instantiation timing with real CLAP plugins.

## Decisions and deviations

- The design started with two restore paths (full replay and diff) plus an
  `UndoExtras` side-car; A-13j deleted the full-replay undo fallback,
  `structurally_compatible`, `UndoFull` and `io.restoring_undo`. There is one
  driver with two callers.
- Stages used to mark points where inline code ran between groups; since
  A-13f nothing runs between stages, so `Stage` only documents the order.
- Code comments still point at `docs/history/refactor/A-13-reconcile.md`; the file now
  lives at `docs/history/refactor/A-13-reconcile.md`.

## Known gaps

- `detach_and_delete_cache_in` can probably fold into
  `detach_and_delete_cache` (noted in the design doc, not done; cleanup only).
