# Arrange-side automation editing

> Stage: **intent**
> Status: **partly built**
> Area: timeline
> Source: gaps found while documenting automation lanes at 8eac71b3; the "show in Arrange" jump from mixer-cleanup.md §3.4 (noted as not built in `resonance-app/src/view/mixer/inspector/automation.rs`).

## Why

Lanes can be drawn and edited on the Arrange timeline, but you cannot manage
them from there:
- A lane can only be created, removed or switched off in the mixer
  inspector's AUTOMATION group. To automate something while arranging, the
  user has to leave the Arrange view.
- Bus and master lanes never show on the Arrange view
  (`target_belongs_to_track` only matches track targets), so a master fade
  cannot be seen against the song.
- Only one breakpoint can be selected (`selected_breakpoint`). There is no
  rubber-band selection, no moving a group of points and no copy/paste.
- The mixer's lane rows have no "show in Arrange" jump.

## What it would do

- An "add lane" picker in the Arrange track header (same options as the
  mixer's `choices_for`), plus Read and remove on each lane sub-row header.
- A read/edit row for bus and master lanes (for example under the global-track
  shelf, or on group-folder rows if those are bus-backed).
- Multi-point selection (rubber band, shift-click), moving a selection as one
  undo entry, and copy/paste of points at the playhead.
- The "show in Arrange" jump from each mixer lane row is its own intent:
  [inspector-automation-jump.md](../mixer/inspector-automation-jump.md).

## What exists to build on

- `AutomationMessage` edit vocabulary and `SetLane` (whole-lane replace) in
  `resonance-app/src/update/automation.rs`.
- The lane sub-rows (`ArrangeRowKind::AutomationLane`,
  `resonance-app/src/state/arrange_layout.rs`) and header cell
  (`resonance-app/src/view/track_header/automation_lane.rs`). This cell
  currently shows only a label.
- The mixer picker: `resonance-app/src/view/mixer/automation.rs`
  (`choices_for`, `add_lane_picker`, `add_lane_message`).
- `AutomationState::lane_selection` and `automation_expanded_tracks` for the
  jump.
- See [../../features/timeline/automation-lanes.md](../../features/timeline/automation-lanes.md).

## Open decisions

1. Where bus and master lanes live on the Arrange view. **Recommended:** one
   collapsible "Busses & master automation" row group below the tracks, using
   the same 44 px lane-row kind.
2. Copy/paste semantics. **Recommended:** paste relative to the playhead, onto
   the same target, replacing points in the pasted span (the `shape`
   range-replace rule).
3. Whether multi-select shares the clip selection model. **Recommended:**
   keep it separate (`selected_breakpoints: Vec`) and clear it on any clip
   press, as today.

## Slices

1. Header controls on lane sub-rows (Read toggle, remove) plus an "add lane"
   picker on the track header, with a golden and a reducer test.
2. Multi-select plus group move (one undo entry).
3. Copy/paste.
4. Bus and master lanes on the Arrange view.

(The "Show in Arrange" jump from the mixer row is tracked in
[inspector-automation-jump.md](../mixer/inspector-automation-jump.md).)

## How we'd know it works

- Goldens in `resonance-app/tests/snapshots/` for the new header controls and
  the bus/master row.
- Reducer tests in `resonance-app/tests/timeline/` for multi-move and paste,
  each asserting one undo entry and the resulting points.
