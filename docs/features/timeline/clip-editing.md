# Clip editing: place, move, split, trim, delete

> Verified against master @ `6137b6fb` on 2026-10-15.
> Spec history: no single spec. Placement came from doc #175 (drag to
> timeline, prototype `docs/intents/prototypes/audio-sample-import-media-browser/`),
> the `clip.*` control methods from docs #265/#275/#276, and the deferral
> rule for edits on a still-loading clip from ba doc #276 BUG 1.

Clips are the blocks on the Arrange lanes. An audio clip plays a slice of a
pooled audio file (or a recorded take); a MIDI clip holds notes. On the
timeline you can drop audio onto a lane, drag a clip along a lane or to
another lane, trim either edge, split an audio clip at the playhead and
delete the selection. An agent can do the same to audio clips through
`clip.*`, with exact sample positions instead of grid snapping.

## Behaviour

### Placing audio

- **Drag from the media browser — built but unreachable.** No view emits
  `DragMessage::Start` today (browser rows only select), so this path runs
  only in tests; see
  [../../intents/ui-shell/media-browser-drag-and-pool-audition.md](../../intents/ui-shell/media-browser-drag-and-pool-audition.md).
  Once started, while the drag hovers over the timeline the canvas shows a
  pill at the cursor, lights the target lane, draws a dashed ghost clip of the
  file's length at the snapped position, and a tooltip with the bar.beat and a
  sample-rate conversion note (for example `→ 48 kHz`) when the file's rate
  differs from the project's. Dropping below the last lane creates a new
  audio track (`"Audio N"`). Dropping on a group header, automation sub-row or
  take sub-row does nothing. The drop becomes one
  `PoolMessage::ImportAndPlace`, so the import and the placement are one undo
  step. The drag preview is session state and is never saved.
- **Drop a file from the OS.** On the Arrange tab, a `wav/flac/mp3/ogg` file
  dropped on the window goes to `PoolMessage::WindowAudioDrop`, which places
  it on a **new audio track at the playhead**. (MIDI files dropped on the
  window take the MIDI-import path instead.)
- **`clip.place`** names the sample by `asset_id` (already pooled) or `path`
  (imported first; a path that matches a pooled asset's original path reuses
  it). The default start is bar 1. The position is **not** snapped. The
  target must be an audio track or an external-instrument track; other track
  kinds are refused because an audio clip on them would never render. It runs
  as a job; for a pooled asset the job is already `done` when the reply
  leaves, with the new `clip_id` allocated by the app.

Import, the pool and relinking are covered in
[audio-pool-and-import.md](audio-pool-and-import.md) and
[media-browser.md](../ui-shell/media-browser-panel.md).

### Move

- Press on a clip body and drag. The start follows the pointer, snapped; the
  lane under the pointer becomes the clip's track (group headers, automation
  and take rows keep the original lane). The mirror updates live; the engine
  gets one `MoveClip` / `MoveMidiClip` on release. One undo step per drag.
- Audio and MIDI clips both move across tracks in the GUI. An audio clip only
  lands on a lane that takes audio clips — an audio or external-instrument
  track (`update/clips.rs::takes_audio_clips`, the rule `clip.move` uses);
  over an instrument, drums or vocal lane it stays on its own lane.
- `clip.move` (audio only) takes `start` and an optional `track_id`, which
  must be an audio or external-instrument track.
- `notes.move_clip` moves a MIDI clip to a 1-based `start_bar` or to a section
  placement's start; it cannot change track. See [midi-editor.md](midi-editor.md)
  for the rest of `notes.*`.

### Trim

- The 6 px band at each clip edge (`theme::CLIP_EDGE_THRESHOLD`) is the trim
  handle. Dragging it trims non-destructively: audio clips change
  `trim_start_frames` / `trim_end_frames`, MIDI clips `trim_start_ticks` /
  `trim_end_ticks`. A left-edge trim moves the clip's start so the audio stays
  where it was.
- The dragged edge snaps to the grid. Minimum length: 10 ms for audio, one
  quarter note for MIDI. MIDI trims convert through the tempo map
  (`sample_to_abs_tick` / `tick_to_abs_sample`) so a trim across a tempo
  change is exact.
- `clip.trim` (audio only) takes `start_offset` / `end_offset` as an
  `AmountSpec` (`beats`, `seconds` or `samples`, exactly one) and an optional
  new `start`. Trimming the head through the API does **not** move the clip;
  pass `start` as well to keep the audio in place. Offsets are clamped so at
  least one frame stays audible. `beats` converts against the tempo map at the
  clip's position.

### Split

- **Split Clip at Playhead** (Cmd/Ctrl+E; Cmd/Ctrl+T in the Logic keymap
  preset; also in the command palette) cuts the **selected audio clip** when
  the playhead is strictly inside it. With no clip selected, it splits the
  comp of a take group under the playhead on the selected track
  ([takes-and-comping.md](takes-and-comping.md)). MIDI clips cannot be split.
- `clip.split` takes `at` (a timeline position strictly inside the clip) and
  returns both ids. The head keeps its id; the tail gets a new app-allocated
  id.
- Both halves are trims of the same source; nothing is copied. The cut edges
  carry no fade: the head's fade-out and the tail's fade-in are set to 0, the
  head keeps the fade-in and the tail keeps the fade-out. The tail drops warp
  markers and vocal tuning (they are keyed to the original clip's timeline).
  The engine applies both halves in one published render graph, so no audio
  block sees one half without the other.

### Delete

- Delete / Backspace (with or without Shift or Cmd) deletes the selected
  global event, else the selected MIDI clip, else the selected audio clip
  (`TimelineDeleteSelection`). It acts only while the timeline owns the
  keyboard (`focus::KeyFocus`); a press on the timeline or a GUI selection
  change aimed at it grants that, a press elsewhere takes it away.
- `clip.delete` (audio only) needs `"confirm": true`; without it the error
  says what would be lost. The pool asset stays.
- Deletes update the mirror at once (not on the `ClipDeleted` echo), so an
  undo pressed before the echo still restores the clip.

### Snap

All pointer edits snap through one function,
`view::timeline::snap::snap_sample_to_grid_tempo`: beats when a bar is at
least 40 px wide, bars when at least 20 px, multi-bar steps below that (the
multi-bar step only on a single-tempo, single-signature project; with a tempo
or signature map it snaps to bars). There is no snap on/off switch and no
modifier to bypass it. The control API never snaps.

### Edits on a clip that is still loading

A clip's audio loads on a worker thread. `MoveClip`, `TrimClip`, `SplitClip`,
`SetClipFade`, `SetClipGain`, `SetClipWarp`, `SetClipWarpMarkers` and
`DetectClipTempo` that arrive before the clip exists in the engine are parked
(`engine::clips::defer_clip_command`) and replayed when it lands. `DeleteClip`
is not parked: it cancels the load.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Engine clip model | `resonance-audio/src/types/clip.rs` | `AudioClip`, `MidiClip`, `split_tail`, `split_head_trim_end`, `duration_frames` |
| Engine handlers | `resonance-audio/src/engine/clips.rs`, `engine/thread/dispatch/clips.rs` | `handle_move_clip`, `handle_split_clip`, `defer_clip_command`, `clip_exists` |
| App mirror | `resonance-app/src/state/clips.rs` | `ClipState`, `MidiClipState`, `ClipDragState`, `ClipTrimState`, `MidiClipDragState`, `MidiClipTrimState`, `ClipEdge` |
| Audio clip reducers | `resonance-app/src/update/clips.rs` | `ClipMessage`, `start/update/end_clip_drag`, `*_clip_trim`, `move_clip_to`, `trim_clip_to`, `split_clip_at` |
| MIDI clip reducers | `resonance-app/src/update/midi_clip.rs` (drag/trim fns live in `update/clips.rs`) | `MidiClipMessage`, `update_midi_clip_trim` |
| Echo handling | `resonance-app/src/engine_events/clips.rs` | `deleted`, `send_mirrored_delete` |
| Drag from browser | `resonance-app/src/state/drag.rs`, `update/drag.rs`, `view/timeline/placement.rs`, `view/timeline/draw/drag.rs` | `DraggedAsset`, `DragPlacement`, `DropResolution`, `DragMessage`, `DropTarget`, `resolve_drop` |
| OS file drop | `resonance-app/src/update.rs` | `arrange_audio_file_drop` → `PoolMessage::WindowAudioDrop` |
| Hit-testing | `resonance-app/src/view/timeline/hit_test.rs`, `view/timeline/input/pointer.rs` | `HitKind`, `hit_test`, `hit_test_audio` |
| Snap | `resonance-app/src/view/timeline/snap.rs` | `snap_sample_to_grid_tempo` |
| Commands | `resonance-app/src/commands/resolve.rs`, `commands/bindings.rs` | `SplitClipAtPlayhead`, `split_target`, `TimelineDeleteSelection`, `timeline_delete` |
| Control API | `resonance-control/src/methods/clip.rs`, `resonance-app/src/update/control/clip.rs` | `PlaceParams`, `MoveParams`, `TrimParams`, `SplitParams`, `DeleteParams`, `AmountSpec`; `place`, `move_clip`, `trim`, `split`, `delete`, `takes_audio_clips` |
| MCP tools | `resonance-mcp/src/tools/clip.rs` | `clip_place`, `clip_move`, `clip_trim`, `clip_split`, `clip_delete` |

Invariants:

- **Ids.** Clip ids come from the app's single allocator
  (`app.media.ids.clips.allocate()`); the control API and the split command
  allocate before dispatch so the reply can name the new clip.
- **Undo.** Pointer gestures are `Begin` on press, `Skip` on move and `Commit`
  on release — one entry per gesture. Discrete edits (`MoveClipTo`,
  `TrimClipTo`, `SplitClipAt`, `DeleteClip`) are `Record`. `ClipMessage::undo_action`
  is exhaustive on purpose.
- **Mirror first.** Handlers write the `ClipState` mirror and then send the
  engine command; `split_clip_at` mirrors both halves at once so
  `song.tracks` sees the tail on the next request.
- Splitting or deleting calls `recompute_pool_usage` so asset usage counts
  stay right.

## Surfaces

- **GUI:** Arrange tab lanes. Drag body = move, drag edge = trim. Cmd/Ctrl+E
  split at playhead; Delete/Backspace delete; double-click a MIDI clip opens
  the piano roll. Audio files dropped from the OS file manager land on a new
  track at the playhead (browser-row drag is not wired, see above).
- **Control API / MCP:** `clip.place` → `mcp__resonance__clip_place`,
  `clip.move` → `mcp__resonance__clip_move`, `clip.trim` →
  `mcp__resonance__clip_trim`, `clip.split` → `mcp__resonance__clip_split`,
  `clip.delete` → `mcp__resonance__clip_delete`. For MIDI clips:
  `notes.create_clip` → `mcp__resonance__notes_create_clip`,
  `notes.move_clip` → `mcp__resonance__notes_move_clip`. All `clip.*` methods
  reject a MIDI clip id with a hint to use `notes.*`.
- **Agent skills:** none of `resonance-agent-plugin/skills/*` call the
  `clip_*` tools.

## How it's tested

| What | Where | Run |
|---|---|---|
| `clip.place/move/trim/delete` (incl. no snap, undo, non-audio track refused, clamps) | `resonance-app/tests/control/control_clip_place.rs` | `./scripts/run-tests.py -p resonance-app` |
| `clip.place` guards (bounce/freeze gate) | `resonance-app/tests/control/control_clip_confirm_place_guard.rs` | same |
| `clip.split` (halves, no fade at cut, refuse outside, one undo, external track) | `resonance-app/tests/control/control_clip_split.rs` | same |
| Beat-unit positions | `resonance-app/tests/control/control_position_beats.rs` | same |
| GUI split at playhead | `resonance-app/tests/timeline/selection_commands.rs` | same |
| Drag snapping with a tempo map | `resonance-app/tests/timeline/clip_drag_tempo_snap.rs`, `snap_signature_map.rs` | same |
| MIDI trim across tempo changes | `resonance-app/tests/midi/midi_clip_trim_tempo.rs` | same |
| Drag-to-timeline reducers | `resonance-app/tests/timeline/drag_placement_handlers.rs` | same |
| Delete echo / undo-before-echo | `resonance-app/tests/timeline/clip_delete_echo_owed.rs`, `undo_before_echo.rs`, `undo_noop_gesture.rs` | same |
| Engine handlers, deferral, load cancel | `resonance-audio/tests/engine/render_graph_clips.rs`, `deferred_clip_commands.rs`, `clip_delete_cancels_load.rs`, `midi_clip_handlers.rs` | `./scripts/run-tests.py -p resonance-audio` |
| OS file drop message | `resonance-app/tests/io/audio_import_entry_points.rs` | same |
| Golden: drag preview | `resonance-app/tests/timeline/drag_placement_visuals.rs` → `tests/snapshots/drag_placement_over_lane_render-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test timeline` |

Not covered: the iced `FileDropped` subscription itself (tests send
`WindowAudioDrop` directly); GUI drag of an audio clip onto a non-audio lane; the Delete key bindings as key
events (the command is tested, not the chord).

## Decisions and deviations

- The GUI snaps, the API does not. `ImportAndPlaceExact` and
  `PlacePooledAsset` exist so a client gets the position it asked for,
  independent of the user's zoom.
- `clip.trim` on the head leaves the clip's start alone, while a GUI left-edge
  trim moves the start. The API keeps "trim" and "move" separate; pass
  `start` to combine them.
- `clip.place` and `clip.move` accept external-instrument tracks (ba doc #275
  P2), so a recorded hardware take can be cut, copied and re-placed.
- There is no rename: a placed clip is named after the source file's stem.

## Known gaps

- MIDI clips have no `notes.*` delete, trim or split, and `notes.move_clip`
  cannot change track. → [midi-clip-arrange-api.md](../../intents/timeline/midi-clip-arrange-api.md)
- No copy, paste or duplicate for clips (Duplicate Selection works on section
  placements only). → [clip-copy-duplicate.md](../../intents/timeline/clip-copy-duplicate.md)
- No snap toggle or snap-bypass modifier.
- No clip rename, in the GUI or the API.
- `song.tracks` reports a clip's start and length but not its trim offsets.
  → [clip-state-readback.md](../../intents/timeline/clip-state-readback.md)
