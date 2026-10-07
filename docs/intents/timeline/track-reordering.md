# Reorder tracks

> Status: **idea**
> Area: timeline (arrange view), mirrored in the mixer
> Source: found in code on 2026-10-07; the track-grouping design (DESIGN.md decision 6) assumes headers can be dragged

## Why

Tracks appear in the order they were created (`TrackState::order`), and
nothing in the GUI or the control API changes that order. A user who adds a
bass after the drums and vocals cannot move it next to the drums, and an
agent building a song cannot lay tracks out in a sensible order.

## What it would do

- Drag a track header up or down the Arrange column to move it; the mixer
  strips follow the same order.
- A control method `track.move(track_id, index)` → MCP `track_move`.
- Undoable as one step; saved with the project.

## What exists to build on

- `TrackState::order`, `TrackRegistry::resort_tracks` and the sorted-order
  invariant in `resonance-app/src/state/tracks.rs`.
- Project reconcile already applies a changed `order`
  (`update/project_io/reconcile/entities.rs`).
- `ArrangeRowLayout` (`state/arrange_layout.rs`) derives rows from the
  sorted tracks, so the canvas and header follow automatically.
- The group membership drag state (`MembershipDragState` in
  `state/interaction.rs`) is a template for a header drag gesture, and the
  two gestures should share one drag (drop between rows = reorder, drop on a
  group = join).

## Open decisions

1. **Group interaction.** Recommended: dropping between two members of a
   group reorders within it (updating `ordered_members`); dropping between
   ungrouped tracks leaves the group.
2. **Engine order.** Recommended: order is UI-only; the engine does not need
   to know, since render order does not affect output.

## Slices

1. Reducer + `track.move` control method + MCP tool + tests.
2. Header drag gesture with an insertion line and golden.

## How we'd know it works

- Control test: `track.move` then `song.tracks` lists the new order; undo
  restores it; save/load keeps it.
- Arrange and mixer goldens after a move.
