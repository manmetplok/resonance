# Finish app-owned entity ids (D-7e, D-7f)

> Stage: **intent**
> Status: **partly built**
> Area: project-core
> Touches: `docs/features/project-core/entity-ids.md`
> Source: Epic D in `docs/history/refactor/refactor-intent.md`; steps D-7e and
> D-7f in `docs/history/refactor/d-6-engine-created-ids.md` §6. Checked
> open in code on 2026-10-07. FU-D7d (`ClearAll` ends a live-MIDI capture)
> shipped 2026-10-15 as BUG-66; see
> [engine-command-loop.md](../../features/audio-engine/engine-command-loop.md).
> Register items covered: ARCH-04 (review 2026-09-26), D-7e, D-7f.

## Why

Epic D's goal is that the app allocates every entity id and the engine never
invents one. D-1…D-5 and D-7a…d landed. Two spaces are still not there:

- **Take-group ids** are still counted by the engine
  (`HandlerState::next_take_group_id`, reset to 1 by `ClearAll`, bumped by
  `RestoreTakeGroups`). Every other space follows the "app allocates, never
  rewinds" rule; this one is reset on every load, and the app cannot name a
  take group before the engine echoes it.
- **The engine's clip counter is dead weight that still runs.**
  `next_clip_id` allocates nothing since D-7d, but `reserve_clip_id` still
  bumps it from several handlers, `SetProjectDir` still scans the bundle's
  WAVs on a worker to raise it (`start_clip_id_scan` /
  `settle_clip_id_scan`), and `DERIVED_CLIP_ID_BASE`'s "only below the
  base" rule plus `tests/engine/derived_clip_id_partition.rs` exist only to
  protect it. Only `CreateMidiClip` refuses a colliding clip id; the other
  create handlers would silently replace a live clip.

Two id authorities is how collisions happened before (ARCH-04).

## What it would do

- Take-group ids come from an app counter (`EntityIds::take_groups`), handed
  to the engine in the existing `GrantIds` grant (`IdGrantBlocks` gains
  `take_groups`); the engine pre-draws them when a loop session opens.
- `grep -n 'next_[a-z_]*_id' resonance-audio/src/engine` returns nothing, and
  an `arch-invariants` test (`engine_has_no_id_counters`) keeps it that way.
- Every engine create handler refuses a colliding clip id with an error
  event; the first clip is untouched.

## What exists to build on

- `resonance-app/src/state/ids.rs`: `IdCounter`, `EntityIds`,
  `send_clip_id_grant`, `refill_clip_id_grant` (the module doc already says
  "D-7e adds a third for take groups").
- `resonance-audio/src/engine/id_grant.rs` (`ClipIdGrant`, the clip grant
  queue), `AudioCommand::GrantIds(IdGrantBlocks)`, built as a struct so take
  groups can be added, and `AudioEvent::IdGrantLow` for the refill.
- `resonance-audio/src/engine/takes.rs`: `resolve_take_group`,
  `restore_take_groups_in_place` (takes both counters today).
- `resonance-audio/src/engine/clips.rs`: `reserve_clip_id`,
  `reserve_clip_ids_in_project_dir`, `reject_if_clip_id_in_use`.
- Tests: `resonance-audio/tests/engine/clip_id_grant.rs`,
  `loop_record_takes.rs`; `resonance-app/tests/io/id_allocation.rs`.
- Engine side of the command loop:
  [engine-command-loop.md](../../features/audio-engine/engine-command-loop.md).
- The D-6 design doc §4.2–4.4 and §6 give the full step list and guards.

## Open decisions

1. Grant size for take groups. Recommended: the design's fixed 64 per grant,
   low-water 32 (decided 2026-09-27, D-6 §7a).
2. What a loop session does when it cannot pre-draw a group id for every
   audio-capturing track. Recommended: the design's all-or-nothing check —
   refuse to start the session, same as an empty clip grant at record start.

## Slices

1. **D-7e:** `take_groups` in `IdGrantBlocks` + app counter seeded on
   restore; delete `next_take_group_id`, `ClearAll`'s reset and the
   `RestoreTakeGroups` bumps. Guards: load → record → new lane id above every
   restored one (`tests/io/take_lanes_persistence.rs`); a same-slot re-run
   reuses its lane and draws no id.
2. **D-7f:** delete `next_clip_id`, `reserve_clip_id`, the `SetProjectDir`
   scan, `DERIVED_CLIP_ID_BASE` in resonance-audio and the partition test;
   add `reject_if_clip_id_in_use` to every create handler; add the
   `arch-invariants` test; rewrite the `state/ids.rs` module doc; per the
   migration plan, delete the fake engine's `clip(None)`.

## How we'd know it works

- The new `arch-invariants` grep test passes and fails when a counter is
  re-added.
- `resonance-app/tests/io/id_allocation.rs` stays green, with its
  leftover `FakeEngine::clip` counter deleted.
- Engine tests: duplicate `LoadMidiClipDirect` / `LoadClipFromWav` →
  `Internal` error, first clip unchanged; take-group grant exhaustion.
- A live session: cycle-record across two project loads; every take lane
  keeps a distinct id and no WAV is overwritten.
