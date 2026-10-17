# Piano-roll note drag as one undo step

> Stage: **intent**
> Status: **idea**
> Area: timeline
> Source: verifying `docs/features/timeline/midi-editor.md` on 2026-10-07.

## Why

Dragging a note in the piano roll sends `MidiEditorMessage::MoveNote`, or
`ResizeNote` for the right edge, on **every** pointer-move event. Both are
classified `UndoAction::Record`. One drag therefore fills the undo history
with dozens of entries, and each one deep-copies the project snapshot.
Undoing a single drag takes many presses. Clip drags in the timeline avoid
this by using `Begin`/`Commit`.

## What it would do

One note drag or resize, from press to release, is one undo entry labelled
"Move note" or "Resize note". A press and release that moves nothing records
nothing.

## What exists to build on

- The `UndoAction::Begin` / `Commit` gesture model (`undo/classify.rs`,
  `commit_undo_gesture`), as used by `MidiClipMessage::StartMidiClipDrag` /
  `EndMidiClipDrag`.
- `view/midi_editor/input.rs`: the drag state (`DragMode::MoveNote` /
  `ResizeNote`) already knows when a drag starts and ends.

## Open decisions

1. Gesture messages or coalescing. Recommended: explicit
   `NoteDragStart`/`NoteDragEnd` messages classified `Begin`/`Commit`, with
   the per-move messages becoming `Skip`. This matches clip drags, and the
   no-op commit check is already there.
2. Whether to send a move on every event or only when the snapped position
   changes. Recommended: only when it changes, to cut engine traffic.

## Slices

1. Move and resize gestures with Begin/Commit, plus a test that a 20-event
   drag leaves one undo entry and that a press and release without movement
   leaves none.

## How we'd know it works

The `resonance-app/tests/midi/` test above passes, and in the app one Ctrl+Z
undoes a whole drag.
