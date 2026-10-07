# Multi-clip selection and marquee

> Status: **idea**
> Area: timeline
> Source: found in code on 2026-10-07 (the Arrange selection holds one clip)

## Why

On the Arrange canvas a user can select one audio clip or one MIDI clip at a
time. There is no Shift/Cmd-click to add clips, no marquee drag and no time
range selection. Moving, deleting or nudging a chorus's worth of clips means
doing each clip separately, with one undo entry each.

## What it would do

- Shift/Cmd-click and a marquee drag on empty lane space build a set of clips
  (audio and MIDI together, across tracks).
- Move, delete, split-at-playhead and nudge act on the whole set as one undo
  entry.
- Copy/paste/duplicate (see [clip-copy-duplicate.md](clip-copy-duplicate.md))
  act on the set once both exist.

## What exists to build on

- `ClipInteractionState` in `resonance-app/src/state/interaction.rs`
  (`selected_clip`, `selected_midi_clip`). The piano roll already has a
  `BTreeSet` multi-selection with marquee (`MidiEditorState::apply_marquee`)
  to copy the pattern from.
- Single-clip move/split/delete reducers in `resonance-app/src/update/clips.rs`
  and `update/midi_clip.rs`; command targets in `commands/resolve.rs`
  (`timeline_delete`, `split_target`, `selection_range`).
- Feature doc: [../../features/timeline/selection.md](../../features/timeline/selection.md).

## Open decisions

1. **One set or two.** Recommended: replace `selected_clip` /
   `selected_midi_clip` with one ordered set of `(kind, ClipId)` — this also
   removes the stale-MIDI-selection bug described in the selection doc.
2. **Marquee start.** Recommended: a drag that starts on empty lane space
   (today a plain press there selects the track); a click without movement
   keeps selecting the track.

## Slices

1. Selection set + Shift-click + group delete, one undo entry.
2. Group move (drag any member) with snap on the grabbed clip.
3. Marquee on the canvas with a golden.

## How we'd know it works

- Timeline tests: select three clips, move, one undo restores all three;
  delete of a mixed audio+MIDI set is one entry.
- A golden of a marquee selection in progress.
