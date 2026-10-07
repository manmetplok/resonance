# Insert / remove bars in the GUI

> Status: **partly built**
> Area: timeline (see `docs/features/timeline/arrangement-bars.md`)
> Source: ba doc #275 P2 built the edit for the control API only

## Why

`ArrangementMessage::InsertBars` / `RemoveBars` move every timeline
collection in one undoable edit, but nothing in the app sends them: there is
no menu item, palette command or ruler gesture. A user who wants to open
four bars for a new intro has to move every clip by hand.

## What it would do

- Ruler context menu: "Insert bars here…" / "Remove bars…", and palette
  commands for both, acting at the bar under the cursor or the playhead.
- A small prompt for the count; for a removal, a confirmation that names what
  would be deleted (the same `removal_casualties` list the control layer
  shows).
- Optionally: a ruler-range selection removes exactly the selected bars.

## What exists to build on

- `resonance-app/src/update/arrangement.rs` (`insert_bars`, `remove_bars`,
  `removal_casualties`), already undo-classified and described in
  `undo/describe.rs`.
- Command registry in `resonance-app/src/commands/` and the marker context menu
  in `view/menus.rs` as a pattern for a ruler menu.

## Open decisions

1. Where: ruler right-click menu plus palette (recommended) vs. Edit menu only.
2. Removal confirmation: always show the casualty list when non-empty
   (recommended), like `track.delete`.

## Slices

1. Palette commands + count prompt + confirm modal; reducer tests.
2. Ruler context menu; golden.
3. Ruler range selection → remove selected bars.

## How we'd know it works

GUI-driven insert produces the same `ShiftOutcome` as the control method on the
same project; undo restores it in one step; goldens for the prompt and menu.
