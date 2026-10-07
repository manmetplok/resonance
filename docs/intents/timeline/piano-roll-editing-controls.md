# Piano-roll editing controls

> Status: **idea**
> Area: timeline
> Source: verifying `docs/features/timeline/midi-editor.md` on 2026-10-07; code comments "Velocity lane: not interactive for now (future: drag velocity bars)" (`view/midi_editor/input.rs`), "The piano roll has no velocity-drag yet" (`update/midi_editor.rs`).

## Why

The piano roll covers basic note entry, but several everyday controls are
missing:

- Velocity can only be changed through `notes.edit`; the velocity lane does
  not take input.
- Zoom is fixed at 0.5 px/tick and snap at a 16th note
  (`open_midi_editor`), so users can't zoom or switch to 8ths or triplets
  while drawing.
- There is no Arrange gesture that creates an empty MIDI clip on an
  instrument track. Clips come only from Compose, recording, MIDI import or
  the control API.

## What it would do

- Drag velocity bars in the lane, for one note or the selected notes.
- Horizontal zoom (Ctrl+wheel or buttons) and a snap picker that reuses the
  `GridChoice` set.
- Double-click, or a draw-mode drag, on an empty instrument lane in Arrange
  creates an empty clip one bar long (or as long as the drag), snapped to
  the grid.

## What exists to build on

- `MidiEditorMessage::SetNoteVelocity` (undoable, frozen-gated).
- `MidiEditorState::{zoom_x, snap_ticks}`.
- `MidiClipMessage::CreateEmptyClip` (used by `notes.create_clip`).
- `GridChoice` in the quantize toolbar.

## Open decisions

1. Velocity drag undo. Recommended: Begin/Commit gesture, as in
   [piano-roll-drag-undo](./piano-roll-drag-undo.md).
2. Whether snap is per editor or per project. Recommended: per editor
   session, kept with `MidiEditorState`.

## Slices

1. Velocity lane drag, with a test.
2. Zoom and snap controls.
3. Create an empty MIDI clip from Arrange.

## How we'd know it works

- Handler tests for each control.
- A golden of the piano roll with the new toolbar; none exists today.
