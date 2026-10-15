# Section placement editing (move, place-again target, clip ids on place)

> Status: **partly built** — fill on place shipped (BUG-73, see
> [sections and placements](../../features/compose/sections-and-placements.md)).
> Area: compose
> Source: code reading during the 2026-10-07 docs reorganisation (`update/compose/section.rs::handle_place`, `view/compose/strip.rs`).

## Why

There is no way to move a placement (GUI or control) other than
inserting/removing bars, and the edit form's "Place again" button places
at bar 0, which fails whenever bar 0 is taken. `section.place` fills the
new placement now, but returns only its `placement_id`, so an agent has to
re-read `song.tracks` to find the clips it got.

## What it would do

- A placement can be moved to another bar (GUI drag in the section band or
  strip, plus `section.move_placement`), carrying its clips, refusing
  overlaps.
- "Place again" places after the last placement (or at the first free
  bar) instead of bar 0.
- `section.place` reports the clip ids the new placement received.

## What exists to build on

- `fill_new_placement` and `lay_take_at_new_placement`
  (`update/compose/section.rs`, `vocal_audio_install.rs`) install a
  placement's material; a move could reuse the per-placement installs.
- `first_free_bar` and `duplicate_target` (`commands/resolve.rs`) already
  find free space.
- `arrangement.insert_bars` moves placements and clips together
  (`update/arrangement.rs`).
- Feature doc: [sections and placements](../../features/compose/sections-and-placements.md).

## Open decisions

1. Vocal audio on a moved placement at a different tempo: keep the take
   and let the tempo-mismatch warning speak, or drop it and read stale
   (what placing does).

## Slices

1. Fix "Place again" target; snapshot the strip.
2. `section.move_placement` control method + GUI drag; tests for overlap
   refusal and clip movement.
3. `PlaceResult` carries the new placement's clip ids.

## How we'd know it works

A moved placement plays the same material at its new bar and undo puts it
back; "Place again" succeeds with bar 0 taken.
