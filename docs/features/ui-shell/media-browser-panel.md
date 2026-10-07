# Media browser panel

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: design doc #175 (epic #35, todos #599–#605); prototype
> `docs/intents/prototypes/audio-sample-import-media-browser/` (DESIGN.md +
> index.html).

In the Arrange view, the **Media** button (or ⌥⌘B) docks a 312 px panel on
the left of the track headers. Users browse folders for audio files, keep
favourite and recent folders, preview ("audition") a file through the
engine without touching the arrangement, and see what the project's media
pool holds. It has three tabs:

- **Files** browses folders on disk, with favourites and recent folders.
- **Pool** lists the project's imported audio assets.
- **Presets** searches the plugin preset library.

An audition transport at the bottom plays the selected file.

## Behaviour

**Showing and hiding**

- The flag is `media.browser.visible` (`BrowserMessage::ToggleVisible`),
  false by default and not saved anywhere. All browser state is session UI
  state: nothing in it is undoable or saved with the project.
- The panel is drawn only in `view_main_area`, which is Arrange only. The
  chrome hides the Media button in other views.
- The Toggle Browser command (⌥⌘B) is not limited to Arrange, but outside
  it only flips the flag.
- The panel header has "MEDIA" and a collapse caret.
- The width is fixed at `theme::BROWSER_WIDTH` (312 px). While the panel is
  open, the track-menu anchor is shifted by it.

**Files tab** (`view/browser/files_tab.rs`)

- A breadcrumb of folder buttons and a star that toggles the folder as a
  favourite.
- A shelf of favourite and recent-folder pills — the way in before any
  folder is open. Opening a folder pushes it onto the recent list, capped
  at 12 (`RECENT_FOLDERS_CAP`).
- A filter field: case-insensitive substring on file names, cleared on
  every navigation.
- The folder listing:
  - Folder rows open the folder.
  - Audio rows show a type glyph, a 48-bucket waveform thumbnail, the name,
    a format chip and the duration. Clicking a row selects (highlights) it.
  - The listing is a `lazy` region keyed on `listing_fingerprint`.
- Folders are scanned off the UI thread (`spawn_blocking`, `scan_folder`).
  The worker reads each file's metadata with
  `resonance_common::scan_audio_folder` and builds its thumbnail with
  `waveform_thumbnail`. A file that fails to decode still gets a row, drawn
  with a flat baseline. A scan result for a folder the user already left is
  dropped.
- An empty folder shows "No audio in this folder" and "Choose files…",
  which opens the OS pool-import picker.
- Favourites and recent folders are per machine. They live in
  `settings.json` → `media`, are loaded at startup into `MediaPool`, and are
  written back straight away on change.

**Pool tab** (`view/browser/pool_tab.rs`)

- One row per pool asset: a glyph (⚠ when missing), a thumbnail, the name,
  a format chip, the `M:SS` duration, and either "used ×N" / "unused" or a
  `relink` chip that opens the
  [relink](../timeline/relink.md) modal.
- While any asset is unused, a "Remove unused" bar appears and asks inline
  first ([audio-pool-and-import.md](../timeline/audio-pool-and-import.md)).
- Apart from the relink chip the rows are plain containers with no press
  handler, so a pool asset can't be selected or auditioned from this tab.

**Presets tab.** This is the media-panel face of the host preset browser
([preset-browser.md](preset-browser.md);
`update::plugin_preset_ui::media_tab_shown`).

**Audition transport**, pinned under every tab:

- Play/Stop, a scrubbable waveform (a Canvas), an `M:SS / M:SS` readout.
- Auto-play, Loop and Sync chips. With Auto-play on, selecting a file
  starts it. Auto-play is UI-only; Loop and Sync are sent to the engine and
  stay set across previews.
- Scrubbing restarts the preview at the chosen frame.
- Engine commands: `AuditionFile { path, start_frame }`, `StopAudition`,
  `SetAuditionOptions { loop_enabled, sync_to_tempo }`.
- While audition plays, the app ticks at full frame rate.

**Engine preview** (`resonance-audio/src/engine/audition.rs`)

- The engine decodes the file off the audio thread, publishes it through
  `audition_source`, and the mixer mixes it into the output.
- Position reports are throttled to about 60 Hz, and `AuditionStopped`
  fires once, at the natural end.
- **Sync is varispeed:** the loop is resampled to fill a whole number of
  beats at the project BPM, which also shifts its pitch
  (`compute_sync_ratio`).
- The preview never moves the main playhead and never adds an undo entry.

**Undo.** Every `BrowserMessage` is `UndoAction::Skip`. Browsing never
touches the project.

**Drag to timeline.** Only the receiving end is built:

- `DraggedAsset::from_entry` holds the file's name, its length at the
  project rate, and a conversion note such as `→ 48 kHz`.
- While a drag is over the timeline, the canvas publishes
  `DragMessage::Hover` with a `DropResolution`: the target lane or the
  "create a new audio track" zone, plus the snapped bar label. The timeline
  draws the drag pill, the lit lane, a ghost clip and the tooltip.
- Esc cancels the drag. A drop becomes `PoolMessage::ImportAndPlace`, one
  undo step.
- **No view sends `DragMessage::Start`.** Only tests do. In the running app,
  files get into the arrangement by OS drag-and-drop onto the Arrange
  window, by "Import audio…" (the OS picker, pool only) or through the
  control API (`pool.import`, `clip.place`).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Panel and tabs | `resonance-app/src/view/browser/mod.rs` | `view_browser_panel`, `tab_body` |
| Files / Pool tabs | `resonance-app/src/view/browser/files_tab.rs`, `pool_tab.rs`, `style.rs` | `files_body`, `listing_fingerprint`, `pool_body`, `audition_transport`, `audition_target`, `WaveThumbnail` |
| State | `resonance-app/src/state/browser.rs`, `state/media.rs`, `state/pool.rs` | `BrowserState`, `BrowserTab`, `FolderScan`, `AuditionState`, `filtered_files`, `MediaState::{browser, drag_placement, pool}`, `MediaPool::{favourites, recent_folders, push_recent_folder, sync_media_browser_settings}` |
| Reducer | `resonance-app/src/update/browser.rs` | `BrowserMessage`, `open_folder`, `start_preview`, `scan_task`, `scan_folder` |
| Drag (receiving end) | `resonance-app/src/update/drag.rs`, `state/drag.rs`, `view/timeline/placement.rs`, `view/timeline/input/pointer.rs` | `DragMessage`, `DraggedAsset`, `DragPlacement`, `DropResolution`, `DropTarget`, `resolve_drop` |
| Folder probe | `resonance-common` | `scan_audio_folder`, `waveform_thumbnail`, `audio_probe::AudioFileEntry` |
| Engine preview | `resonance-audio/src/engine/audition.rs`, `engine/thread/dispatch/audition.rs` | `AuditionFile`, `StopAudition`, `SetAuditionOptions`, `compute_sync_ratio` |
| Width | `resonance-app/src/theme.rs` | `BROWSER_WIDTH` |
| Settings | `resonance-app/src/settings.rs` | `MediaBrowserSettings` |

View-performance rule: the audition playhead is a live readout. The
transport is rebuilt every frame, and the fingerprint of the scrub canvas
includes the position (`audition_scrub_fingerprint` test). The Files listing
sits in a `lazy` region keyed on the scan.

Pool import, relink and remove-unused are documented in
[audio-pool-and-import.md](../timeline/audio-pool-and-import.md) and
[relink.md](../timeline/relink.md).

## Surfaces

- **GUI:** the Arrange chrome's **Media** button; ⌥⌘B; the panel's caret.
- **Control API / MCP:** there are no browser or audition methods. The pool
  is reachable through `pool.list` / `pool.import` / `pool.remove_unused` →
  `mcp__resonance__pool_list`, `mcp__resonance__pool_import` and
  `mcp__resonance__pool_remove_unused`.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Navigation, filter, favourites and recents, tabs, audition messages | `resonance-app/tests/io/browser_handlers.rs` | `./scripts/run-tests.py -p resonance-app` |
| Audition event mirror | `resonance-app/tests/io/engine_events_pool_mirror.rs` | same |
| Listing `lazy` key and thumbnail cache; scrub fingerprint | `resonance-app/tests/io/files_listing_fingerprint.rs`, `tests/control/audition_scrub_fingerprint.rs` | same |
| Media settings sync | `resonance-app/tests/io/pool_persistence.rs` | same |
| Drag handlers + visuals (from synthetic `Start`) | `resonance-app/tests/timeline/{drag_placement_handlers,drag_placement_visuals}.rs` | same |
| Goldens | `media_browser_{hidden,files_tab,pool_tab}`, `files_tab_{populated,empty}`, `pool_tab_{populated,remove_unused_confirm}`, `audition_transport_{idle,playing}`, `drag_placement_over_lane_render` (from `io/media_browser_scaffold.rs`, `io/files_tab_rendering.rs`, `io/pool_tab_rendering.rs`, `plugins/audition_transport_snapshot.rs`, `timeline/drag_placement_visuals.rs`) | `RESONANCE_BLESS=1 …` |
| Engine preview | `resonance-audio/tests/mixer/audition_preview.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Probe / thumbnails | `resonance-common/tests/audio_probe.rs` | `./scripts/run-tests.py -p resonance-common` |

Not covered: a real pointer drag from a Files row, because the GUI has
none; auditioning a pool asset from the GUI; audible output of the preview
in a live session (the app tests assert the `AudioCommand`s).

## Decisions and deviations

- The panel is docked rather than modal, so browsing never covers the
  arrangement; audio uses the WARM colour domain; the audition transport
  sits at the bottom — all as designed (doc #175).
- The design's primary placement gesture was dragging from the panel to the
  timeline. As built, the app has the drop side but not the drag source.
- Pool rows were drawn as plain containers; the design's audition and drag
  of pool rows was not wired. `audition_target` can resolve a pool asset,
  but no row ever selects one.
- Sync-to-tempo is varispeed, because the audition path has no
  pitch-preserving stretch.
- A third tab, Presets, was added by the plugin preset library spec (slice
  P6). Some doc comments still say "two tabs" (`state/browser.rs`,
  `update/browser.rs::SelectTab`).

## Known gaps

- Dragging from Files or Pool rows to the timeline is not wired, and pool
  rows can't be selected or auditioned
  ([intent](../../intents/ui-shell/media-browser-drag-and-pool-audition.md)).
- Pool rows draw flat thumbnails after a project reload (peaks are not saved
  or rebuilt).
- The Files tab has no sorting and no multi-select import from the listing.
