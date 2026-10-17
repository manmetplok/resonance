# Drag from the media browser to the timeline; audition pool assets

> Stage: **intent**
> Status: **partly built**
> Area: ui-shell
> Touches: media browser / timeline (drop)
> Source: design doc #175 (todo #605) and `docs/intents/prototypes/audio-sample-import-media-browser/DESIGN.md` ("drag-to-timeline is the primary placement gesture"); code check at `8eac71b3`.

## Why

In the design, the media browser's main job is "browse, audition, drag
onto the arrangement". As built, a user can browse and audition files, but
can't drag a row anywhere:

- No view emits `DragMessage::Start`.
- The Pool tab's rows have no press handler at all, so a pool asset can't
  even be auditioned.

To get a sample into the song, the user has to leave the panel and use an OS
file drag or "Import audio…". "Import audio…" imports into the pool only,
and the GUI then offers no way to place the imported asset; the only GUI
placement left is dropping a file from the OS onto the window, which always
makes a new track at the playhead.

## What it would do

- Pressing and dragging an audio row in **Files** or **Pool** starts a
  drag:
  - a pill follows the pointer;
  - the timeline shows its existing lane highlight and ghost clip;
  - releasing on a lane places the file at the snapped position as one undo
    step; releasing on the "new audio track" zone creates a track first;
  - a Pool row places the existing asset without importing it again;
  - releasing outside the timeline, or pressing Esc, cancels.
- Clicking a Pool row selects it. Auto-play auditions it, as in Files.

## What exists to build on

- `resonance-app/src/update/drag.rs`: `DragMessage::{Start, Hover, Drop,
  Cancel}`. Drop re-dispatches `PoolMessage::ImportAndPlace` as one undo
  step.
- `state/drag.rs`: `DraggedAsset::from_entry`, `DragPlacement`,
  `DropResolution`; `view/timeline/placement.rs::resolve_drop`.
- `PoolMessage::PlacePooledAsset` (`update/pool.rs`) places an
  already-pooled asset with no import, today sent only by `clip.place`.
  Its doc comment says a GUI pool-row drag "always goes through
  `ImportAndPlace`, which short-circuits" — both halves are wrong: there is
  no pool-row drag, and `import()` always allocates a fresh asset id and
  re-imports the file.
- The timeline already emits Hover/Drop (`view/timeline/input/pointer.rs`)
  and draws the preview (golden `drag_placement_over_lane_render`).
- Esc cancels (`view/timeline/input/keyboard.rs`).
- The preset drag (`PresetUiMessage::MediaPress` / `DragMoved` / `DragEnd`
  and the root `mouse_area` wrapper in `view/mod.rs`) is a working pattern
  for a drag that starts in the panel and ends on another surface.
- `view/browser/pool_tab.rs::audition_target` already resolves pool assets
  for the audition transport.
- [media-browser-panel.md](../../features/ui-shell/media-browser-panel.md),
  [audio-pool-and-import.md](../../features/timeline/audio-pool-and-import.md).

## Open decisions

1. Should the drag start on press-and-move past a threshold, or on press?
   **Recommend a 4 px threshold**, as the preset drag does, so a click still
   just selects.
2. Should dropping below the last track create a new audio track? **Recommend
   yes.** The design shows a "new audio track" drop zone, and the OS-drop
   path already creates one.
3. Should a drag carry several selected files? **Recommend one file first.**
4. What a Pool-row drop sends. **Recommend** a drop-target-aware variant of
   `PlacePooledAsset` that snaps like `ImportAndPlace` — not
   `ImportAndPlace` with the original path, which imports the file again.
   Fix the misleading `PlacePooledAsset` comment in the same slice.

## Slices

1. Make Pool rows selectable and auditionable. Add a handler test and a
   golden of the selected row.
2. Start a drag from a Files row: emit `DragMessage::Start` from a
   `mouse_area` press plus a threshold, and use the root wrapper to follow
   and end it. Add a test that drives the Simulator from row press to lane
   release and asserts one placed clip and one undo entry.
3. Start a drag from a Pool row (an already-pooled asset, no import);
   assert no new asset id is allocated and the pool size is unchanged.
4. New-track drop zone.

## How we'd know it works

- An `iced_test::Simulator` test in `resonance-app/tests/io/` drags a row
  onto a lane.
- The existing `timeline/drag_placement_handlers.rs` tests keep passing.
- By hand: drag a WAV from the panel during playback.
