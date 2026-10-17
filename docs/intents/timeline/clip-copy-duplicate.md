# Copy, paste and duplicate clips

> Stage: **intent**
> Status: **idea** (the ⌘D command exists, but only for section placements)
> Area: timeline
> Touches: clip copy / ui-shell (command binding)
> Source: gap found while documenting clip editing (2026-10-07);
> `docs/history/specs/command-palette.md` §5.3 as-built note; still true at
> `8eac71b3`.

## Why

There is no way to copy a clip on the Arrange view. Duplicate Selection
(Cmd/Ctrl+D) only duplicates a section placement into the bars right after
it; with an audio or MIDI clip selected the command shows as unavailable
with "Select a section with room after it". There is no copy/paste command
and no modifier-drag copy. Repeating a recorded take or a MIDI pattern —
one of the most common arranging moves in any DAW — means re-recording it,
re-dropping the file from the browser, or (for agents) `clip.place` with an
`asset_id` — which a recorded take does not have (its `asset_ref` is
`None`), and MIDI clips need `notes.create_clip` plus re-inserting every
note.

## What it would do

- With one or more clips selected, Cmd/Ctrl+D places a copy of each right
  after the selection's end, on the same track. The new copies become the
  selection, so pressing it again keeps extending. The section-placement
  behaviour stays as it is when a placement is selected.
- Copy / paste at the playhead on the selected track; optionally
  Alt/Option-drag to copy.
- Works for audio clips (sharing the same source; nothing is copied on disk)
  and MIDI clips (notes copied). Trims, fades, gain and warp settings come
  along.
- The whole duplicate is one undo entry.
- A control method, e.g. `clip.duplicate { clip_id, start?, track_id? }`
  (MCP `clip_duplicate`), returning the new id.

## What exists to build on

- `ClipMessage::SplitClipAt` already creates a second clip over the same
  source with an app-allocated id (`resonance-app/src/update/clips.rs`,
  `AudioClip::split_tail`); a duplicate is the same shape without the trim.
- `resonance-app/src/commands/resolve.rs`: `DuplicateSelection` resolves
  through `duplicate_target` to `ComposeMessage::PlaceSection`;
  `selection_range` already computes the selection span for Loop Selection.
- Commands that create entities allocate ids in `CommandId::build(&mut
  Resonance)`, as Split Clip does. Clip id allocation:
  `app.media.ids.clips.allocate()`.
- The only existing clip-placing primitive, `PoolMessage::PlacePooledAsset`,
  places the whole asset and would drop trims, fades and gain — which is why
  the command-palette spec deferred this.
- Related: [clip-editing.md](../../features/timeline/clip-editing.md),
  [selection.md](../../features/timeline/selection.md),
  [clip-multi-select.md](clip-multi-select.md).

## Open decisions

1. **Engine command.** Recommended: a `DuplicateClip { clip_id, new_clip_id,
   start_sample, track_id }` that shares the `ClipSource` `Arc` (the same
   pool asset), rather than reloading the WAV or copying audio. A clip is a
   window onto an asset already.
2. **Shortcut.** Recommended: Cmd/Ctrl+D duplicates the selected clip(s)
   when any are selected, else the section placement (current behaviour).
3. **Target span overlaps an existing clip.** Recommended: unavailable with
   a reason, matching the section case, rather than overwriting.
4. **Control method in the same slice?** Recommended: yes, per the
   vertical-slice rule.

## Slices

1. Audio clip duplicate: engine command, a `ClipMessage::DuplicateClips {
   ids, offset }` reducer copying every clip field, ⌘D binding through
   `build()`, `clip.duplicate` + MCP tool, tests (one undo entry, fields
   preserved).
2. MIDI clip duplicate, copying notes.
3. Multi-selection across tracks.
4. Copy/paste at playhead.

## How we'd know it works

- Reducer tests in `resonance-app/tests/timeline/selection_commands.rs`.
- A control round-trip test: one undo entry; undo restores the arrangement
  exactly.
- An engine render test that the new clip plays the same frames; pool usage
  count rises by one.
