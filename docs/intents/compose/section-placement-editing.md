# Section placement editing (fill on place, move, place-again)

> Status: **partly built**
> Area: compose
> Source: code reading during the 2026-10-07 docs reorganisation (`update/compose/section.rs::handle_place`, `view/compose/strip.rs`).

## Why

Placing a section that already has generated parts gives an **empty**
placement: `handle_place` records the placement but derives nothing, so
the new chorus is silent until something regenerates each lane. An agent
that does `section.place` then `song.notes` finds no clips. There is also
no way to move a placement (GUI or control) other than inserting/removing
bars, and the edit form's "Place again" button places at bar 0, which
fails whenever bar 0 is taken.

## What it would do

- Placing a definition installs its existing material (derived MIDI for
  every lane with clips, drum clips, installed vocal audio) at the new
  placement in the same undo step.
- A placement can be moved to another bar (GUI drag in the section band or
  strip, plus `section.move_placement`), carrying its clips, refusing
  overlaps.
- "Place again" places after the last placement (or at the first free
  bar) instead of bar 0.

## What exists to build on

- `install_derived_midi_clip`, `rederive_section_clips` (only lanes that
  have clips), `materialize_drum_clips_for(Some(def))`, vocal
  `placement_audio_starts` in `update/compose/`.
- `first_free_bar` and `duplicate_target` (`commands/resolve.rs`) already
  find free space.
- `arrangement.insert_bars` moves placements and clips together
  (`update/arrangement.rs`).
- Feature doc: [sections and placements](../../features/compose/sections-and-placements.md).

## Open decisions

1. Re-derive or copy on place? Recommended: re-derive via
   `rederive_section_clips` — material is a pure function of the
   definition, and it keeps the ids per slot.
2. Vocal audio on a new placement at a different tempo: install the
   existing WAV and let the tempo-mismatch warning speak (recommended),
   or force a re-render.

## Slices

1. Fill on place: call the re-derive path from `handle_place`; control
   `section.place` returns the new clip ids; test in
   `tests/compose/` + `tests/control/control_section_harmony.rs`.
2. Fix "Place again" target; snapshot the strip.
3. `section.move_placement` control method + GUI drag; tests for overlap
   refusal and clip movement.

## How we'd know it works

`section.place` followed by `song.notes` on the returned clip ids shows the
same notes as the first placement; undo removes placement and clips
together.
