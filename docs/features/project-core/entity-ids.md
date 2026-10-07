# Entity ids

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/refactor/D-6-engine-created-ids.md` (design,
> §7a holds the human decisions), `docs/history/refactor/refactor-intent.md`
> (Epic D), `docs/history/refactor/arch-migration-plan.md` → ARCH-04, "D-7b
> landed", "D-7d landed".

Tracks, busses, plugins, sends, clips, pool assets, references, markers and
automation lanes are all named by numeric ids that the app, the engine, the
project file, undo snapshots and control clients share. This feature is the
rule for who hands those ids out, so two things never get the same id and an
id that once named a file is never reused.

## Behaviour

- **The app allocates almost everything.** Every add the app sends carries a
  concrete id; the engine honours it or refuses a collision with an error
  event (`EngineErrorKind::Internal`) rather than replacing the live entity.

| Space | Allocator | Notes |
|---|---|---|
| Track (and track group, same space) | `Resonance::allocate_track_id` (`state/ids.rs`) | starts at 1; skips ids held by tracks *or* groups; `debug_assert`s it stays below `BUS_ID_BASE` |
| Bus | `TrackRegistry::allocate_bus_id` (`state/tracks.rs`) | starts at `BUS_ID_BASE` = 2 000 000 000, so `song.tracks` (tracks and busses in one id-addressed list) never shadows a bus |
| Plugin instance | `Resonance::allocate_plugin_id` (`state/plugin_index.rs`) | engine has no counter |
| Aux send | `AuxSendState::allocate_send_id` (`state/aux_sends.rs`) | only `AddAuxSend` is refused on collision; `SetAuxSend` reuses ids by design |
| Reference | `ReferenceState::alloc_engine_id` (`reference/state.rs`) | missing references use a separate range from `MISSING_REFERENCE_ID_BASE` = 1e9 and never reach the engine |
| Clip (audio and MIDI, one space) | `EntityIds::clips` on `MediaState::ids` | starts at `CLIP_ID_BASE` = 2^40; **never reset** — not by undo, `ClearAll` or a load |
| Pool asset | `EntityIds::assets` on `MediaState::ids` | app is the only allocator; the engine refuses to overwrite an existing `asset_<id>.wav` |
| Marker, automation lane, groove | their own app-side counters | the engine never sees these ids |
| Take group | **engine** `HandlerState::next_take_group_id` | still engine-owned (D-7e open) |

- **Engine-created clips use a grant.** Recordings, cycle-record passes,
  live-MIDI captures and realtime bounces need an id at a moment the app
  cannot decide. The app hands the engine blocks of its clip counter
  (`AudioCommand::GrantIds`, `CLIP_GRANT_SIZE` = 1024): once at startup
  (`Resonance::new`) and as the last reconcile domain of every disk load.
  The engine reports `AudioEvent::IdGrantLow` below 512 left and the app
  refills (unless a load is in flight). `ClearAll` revokes the grant. The
  engine skips a granted id whose WAV already exists.
- **Out of ids:** a record start rolls without recording and reports `Busy`;
  a loop seam keeps the finished pass and stops that track; live MIDI drops
  the note with one error per run.
- **Engine clip names** are bare kinds ("Recording", "MIDI Take", "Take");
  the app numbers them per track from existing names
  (`number_engine_clip_name`), so the large ids never show.
- **Seeding.** Counters are raised, never lowered: past every id in a loaded
  project or restored snapshot, and on load or Save As into an existing
  bundle past every `audio/clip_<id>.wav` and `audio/asset_<id>.wav` on disk
  (`seed_clip_ids_on_disk`, `seed_asset_ids_on_disk`). This is what makes
  "an id that ever named a file is never reissued in the session" (STATE-08)
  hold.
- **Leftover engine clip counter.** `HandlerState::next_clip_id` allocates
  nothing any more, but `reserve_clip_id` still bumps it (only for ids below
  the base) from several handlers, and `SetProjectDir` still scans the
  bundle's WAVs to raise it. D-7f deletes all of this.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Bases, skip loop, counters | `resonance-app/src/state/ids.rs` | `CLIP_ID_BASE`, `BUS_ID_BASE`, `MISSING_REFERENCE_ID_BASE`, `allocate_unused`, `IdCounter` (`allocate`, `allocate_block`, `seed_past`), `EntityIds`, `seed_clip_ids_on_disk`, `seed_asset_ids_on_disk`, `send_clip_id_grant`, `refill_clip_id_grant`, `allocate_track_id` |
| Grant in the restore | `resonance-app/src/update/project_io/reconcile/restored.rs` | `ClipIdGrant` |
| Engine clip naming | `resonance-app/src/engine_events/mod.rs` | `number_engine_clip_name`, `ENGINE_CLIP_KINDS` |
| Engine grant | `resonance-audio/src/engine/id_grant.rs` | `ClipIdGrant` (engine side), `take_unused_wav` |
| Wire types | `resonance-audio/src/types/{mod.rs,commands.rs,events.rs}` | `DERIVED_CLIP_ID_BASE`, `CLIP_GRANT_SIZE`, `CLIP_GRANT_LOW_WATER`, `GrantIds`, `IdGrantBlocks`, `IdGrantLow` |
| Leftover engine counters | `resonance-audio/src/engine/{clips.rs,takes.rs,tracks.rs,thread/mod.rs}` | `next_clip_id`, `reserve_clip_id`, `reserve_clip_ids_in_project_dir`, `next_take_group_id`, `resolve_take_group` |

Invariants: a new entity type gets one allocator, on the app side, and the
engine refuses a missing or colliding id. No counter is ever lowered. A new
fixed base must keep the `const _` ordering assert in `ids.rs` true.

## Surfaces

- **Control API / MCP:** ids returned by every create call (`track_id`,
  `clip_id`, …) come from these allocators; a reply can carry the id before
  the engine echoes because the app picked it.

## How it's tested

| What | Where | Run |
|---|---|---|
| No collisions across spaces | `resonance-app/tests/io/id_allocation.rs` | `./scripts/run-tests.py -p resonance-app` |
| Startup track reserves id 1 | `resonance-app/tests/io/startup_default_track.rs` | same |
| Derived clips + counter across reopen | `resonance-app/tests/io/derived_clip_lifecycle.rs` | same |
| Engine grant, duplicate rejection | `resonance-audio/tests/engine/{clip_id_grant,create_midi_clip_mandatory_id,track_id_duplicate_rejected,bus_id_duplicate_rejected,aux_send_id_duplicate_rejected}.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Old partition rule (to delete in D-7f) | `resonance-audio/tests/engine/derived_clip_id_partition.rs` | same |

Not covered: a duplicate `LoadMidiClipDirect` or `LoadClipFromWav` being
refused (only `CreateMidiClip` checks `reject_if_clip_id_in_use` today; D-7f
extends it to the other create handlers), and an `arch-invariants` test that
the engine holds no id counter.

## Decisions and deviations

- The D-6 design rejected the original "reserve a block per `ArmTrack`" plan
  (loop passes are unbounded, live MIDI has no record start) in favour of a
  fixed-size, refillable grant.
- Clip numbering in names is per track and derived from names, not a counter
  (D-6 §7a).
- `BUS_ID_BASE` stays although the engine has no bus counter, because of the
  shared `song.tracks` id list.

## Known gaps

- Take-group ids are still engine-allocated, and the engine's clip counter,
  `reserve_clip_id`, the `SetProjectDir` WAV scan and the partition test are
  still there. See [finish-app-owned-ids](../../intents/project-core/finish-app-owned-ids.md).
- `ClearAll` does not clear the engine's `midi_recording` map (FU-D7d), so a
  live MIDI capture open across a project load can target a clip that is gone.
