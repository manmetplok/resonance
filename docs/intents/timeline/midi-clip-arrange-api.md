# MIDI clip arrangement edits over the control API

> Status: **idea**
> Area: timeline (control-api surface)
> Source: gap found while documenting clip editing (2026-10-07).

## Why

An agent can create a MIDI clip (`notes.create_clip`) and move it in time
(`notes.move_clip`), but cannot delete it, trim it, split it or move it to
another track. `clip.*` refuses MIDI clip ids, and the `clip_delete` MCP tool
description says MIDI clips "are deleted through the notes.* surface" — no
such method exists (`notes.delete` deletes one note). A wrong clip made by an
agent can only be removed with `edit_undo` or by the user.

## What it would do

- Delete a MIDI clip (with the `confirm` convention, since notes are lost).
- Move a MIDI clip to another instrument / drums / vocal track.
- Trim either edge, in beats.
- Optionally split a MIDI clip at a position (the GUI cannot either).

## What exists to build on

- GUI reducers: `MidiClipMessage::{DeleteMidiClip, MoveClipTo}` and the
  drag/trim fns in `resonance-app/src/update/clips.rs`
  (`end_midi_clip_drag`, `update_midi_clip_trim` with tempo-map-aware tick
  conversion).
- Engine commands: `MoveMidiClip` (with `new_track_id`), `TrimMidiClip`,
  `DeleteMidiClip` (see `resonance-audio/tests/engine/midi_clip_handlers.rs`).
- Control handlers for notes: `resonance-app/src/update/control/notes.rs`;
  method names in `resonance-control/src/methods/notes.rs`.
- [clip-editing.md](../../features/timeline/clip-editing.md).

## Open decisions

1. **`notes.*` or widen `clip.*`?** Recommended: widen `clip.move`,
   `clip.trim`, `clip.delete` (and `clip.split`) to accept MIDI clip ids, so
   one surface edits clips of either kind; keep `notes.move_clip` as is.
   Trim amounts for MIDI accept `beats` only.
2. **Derived clips.** Section-generated MIDI clips are regenerated from the
   section; decide whether deleting one is allowed or redirected to the
   section's lane generator. Recommended: allow it, matching the GUI.

## Slices

1. `clip.delete` for MIDI clips + MCP description fix + test.
2. `clip.move` with `track_id` for MIDI clips (instrument-kind tracks only).
3. `clip.trim` for MIDI clips.
4. MIDI split (GUI command and API together).

## How we'd know it works

Tests in `resonance-app/tests/control/` per slice: one undo entry per call,
the right track-kind refusals, and `song.tracks` reflecting the change on the
next request.
