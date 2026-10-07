# Audio pool and file import

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/intents/prototypes/audio-sample-import-media-browser/DESIGN.md` (doc #175, epic #35).

The project's media pool holds every audio file brought into the project.
Importing a file (WAV, FLAC, MP3, OGG) copies it into the project folder as a
stereo WAV at the project rate, and the pool shows each asset with its
format, length and how many clips use it. Users import from the
"Import audio…" button, by dropping files on the Arrange window, or from the
media browser. An agent imports with `pool.import` and places with
`clip.place`. The same area also hosts the Standard MIDI File import dialog,
which is described at the end.

## Behaviour

### Importing audio

- **Accepted files:** `wav`, `flac`, `mp3`, `ogg`, matched on the extension
  without regard to case (`POOL_AUDIO_EXTENSIONS`).
- **A saved project is required.** With no project folder the import is
  refused with the banner "Save the project before importing audio, so
  imported files have a home."
- **What the engine does per file:** it decodes the file, mixes it to stereo,
  resamples it to the project rate, and writes `audio/asset_<id>.wav` with
  `create_new`, so it never overwrites an existing file. It then computes
  waveform peaks. A batch runs on one short-lived worker thread, one file
  after another. A file that fails or panics only fails its own row, and the
  rest of the batch carries on.
- **Ids:** the app allocates every asset id before it sends
  `ImportAudioToPool` (D-7a). The allocator only counts upward within a
  session. At load it is moved past both the loaded project's ids and any
  `asset_<id>.wav` file found on disk, so a new import can never take an id
  that is already in use.
- **Progress modal:** every import, including one started through the control
  API, opens the transcode-progress modal. Each file shows as Queued, then
  Working, then Done or Failed with the reason. A new import replaces the
  rows of the last one. The modal blocks clicks until every file has
  finished, then shows a Done button.
- **Entry points:**
  - *Import audio…* (transport chrome button, the Files tab's
    "Choose files…", the command palette's "Import Audio to Pool…",
    `Cmd+Shift+I`) adds files to the pool **without placing them**.
  - *OS file drop on the window* while the Arrange view is showing creates a
    **new audio track** named `Audio N`. One clip is placed per dropped file,
    at the playhead snapped to the grid. iced reports no cursor position for
    an OS drop, so the drop point is ignored.
  - *Browser drag-to-timeline* (`DragMessage` → `PoolMessage::ImportAndPlace`)
    is fully handled in update, but nothing in the view starts the drag. See
    [media-browser.md](../ui-shell/media-browser-panel.md).
- **Placement** waits until the file's `AssetImported` event arrives. The
  queued `PendingImport` is matched by asset id. If the target track was
  deleted in the meantime, the placement is dropped. Several files dropped on
  one lane all land stacked at the same snapped position.
- **Undo:** the whole import is one undo entry. It is recorded before the
  command is sent and covers the pool asset(s), the placed clip(s) and any
  track created for a new-track drop. When an undo goes past the import, the
  queued placements are dropped (`PendingImports::drop_undone`). A project
  load clears them as well.

### The pool

- `MediaPool::assets` keeps import order. Usage counts are derived from
  `ClipState::asset_ref` by `recompute_pool_usage` and are never stored.
- If the same id arrives with a different file, the existing asset is kept
  and a warning is logged (`MediaPool::add`). The pool never silently points
  existing clips at different audio.
- **Remove unused** (Pool tab, `pool.remove_unused`) takes every asset that no
  clip uses out of the index, except an asset whose relink is still running.
  The WAV files stay in `audio/`, so one undo brings the assets back exactly.
- **Missing assets:** at load, an asset whose `asset_<id>.wav` file is absent
  is kept and flagged `missing`. See [relink.md](./relink.md).
- **Persistence:** `ProjectFile::pool_assets` (`ProjectPoolAsset`) stores id,
  paths, format, channel count, source rate and length.
  `ProjectClip::asset_ref` links each clip to its asset. Thumbnail peaks are
  **not** saved.
- Favourites and recent folders also live on `MediaPool`, but they are
  stored in `settings.json` (user-level state), not in the project.

### MIDI file import (sibling flow)

The Import MIDI dialog (`ImportDialogState`, transport chrome button, the
palette's "Import MIDI…", or dropping a `.mid`/`.midi` file on the window)
works in stages: Drop → Parsing → Review → TempoConflict → Imported (or
Error).

- Review lists each track of the file with its note count, pitch range and a
  preview. Tracks with notes start selected. The conductor track can't be
  selected. A track that uses channel 10 is labelled Drum, every other track
  Instrument. Users can rename a track and choose where the import starts
  (bar 1 or the playhead) and whether it makes new tracks or merges into an
  existing instrument or vocal track.
- If any tempo in the file differs from the project tempo, the dialog stops
  at TempoConflict and asks for one of three choices:
  - keep the project tempo and match bars;
  - keep the project tempo and match time (notes are re-timed onto the
    project tempo map);
  - adopt the file's tempo from the placement bar onward.
- Confirm is a single undo entry. `confirm_blocker` refuses when nothing is
  selected, when no merge target is chosen, when the merge target is missing
  or the wrong kind of track, or when it is frozen. Clips are rounded up to
  whole bars. Confirm writes notes through `create_clip_with_notes`, which
  `notes.import_midi` also uses.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Pool model | `resonance-app/src/state/pool.rs` | `PoolAsset`, `AssetRef`, `MediaPool`, `remove_unused_pool_assets`, `relink_clip` |
| Pending placements + progress | `resonance-app/src/state/pool_import.rs` | `PendingImports`, `PlacementTarget`, `ImportProgressTracker` |
| Grouping | `resonance-app/src/state/media.rs` | `MediaState` (`pool`, `pool_import`, `import_progress`, `ids`) |
| Import orchestration | `resonance-app/src/update/pool.rs` | `PoolMessage`, `import`, `resolve_target`, `place_pooled_asset` |
| Engine-event mirror | `resonance-app/src/engine_events/pool.rs` | `asset_imported`, `import_failed`, `place_clip_with_id` |
| Engine import | `resonance-audio/src/engine/import_pool.rs` | `import_one_to_pool`, `run_pool_import_with`, `handle_import_audio_to_pool` |
| Window drop | `resonance-app/src/update.rs` | `arrange_audio_file_drop`, MIDI `file_drops` |
| Persistence | `resonance-app/src/project/model.rs`, `update/project_io/replay/restore.rs` | `ProjectPoolAsset`, `restore_pool_assets` |
| Id allocators | `resonance-app/src/state/ids.rs` | `seed_asset_ids_on_disk` |
| Progress modal | `resonance-app/src/view/import_progress_dialog.rs` | dismissed by `UiMessage::DismissImportProgress` |
| MIDI import | `resonance-app/src/state/import.rs`, `update/import.rs`, `view/import_dialog.rs` | `ImportDialogState`, `ImportMessage`, `confirm`, `retime_notes`, `adopted_tempo` |
| Control | `resonance-app/src/update/control/clip.rs` | `list`, `import`, `remove_unused`, `place` |

Invariants:

- The app is the only allocator of asset ids.
- Placement is keyed by asset id, never by path.
- Engine events from the import never record undo; the import's single
  entry is recorded up front.

## Surfaces

- **GUI:** Import audio… / Import MIDI… buttons in the transport chrome, the
  Media browser's Pool tab, OS file drop on the Arrange view, command palette
  (`ImportAudio` `Cmd+Shift+I`, `ImportMidi`).
- **Control API / MCP:**
  - `pool.list` → `mcp__resonance__pool_list`
  - `pool.import` (a job; at most 64 files) → `mcp__resonance__pool_import`
  - `pool.remove_unused` (needs `confirm`) → `mcp__resonance__pool_remove_unused`
  - `clip.place` → `mcp__resonance__clip_place`, see [clip-editing.md](./clip-editing.md).
    It reuses an asset whose `original_path` matches, whereas `pool.import`
    always imports the file again.
  - `notes.import_midi` → `mcp__resonance__notes_import_midi`, see [midi-editor.md](./midi-editor.md).
- **Agent skills:** `mixing` (references/reading-meters.md) and `mastering`
  use `pool_import`/`pool_list` to load reference tracks. `arranging` and
  `drumming` use `notes_import_midi`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Engine per-file import, ordering, panics | `resonance-audio/tests/io/import_audio_to_pool.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Entry points, placement, event mirror | `resonance-app/tests/io/{audio_import_entry_points,import_placement,engine_events_pool_mirror,id_allocation}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Persistence, Remove unused | `resonance-app/tests/io/{pool_persistence,pool_remove_unused}.rs` | same |
| Usage counts across edits | `resonance-app/tests/control/pool_usage_staleness.rs` | same |
| Control `pool.*` / `clip.place` | `resonance-app/tests/control/{control_clip_place,control_clip_confirm_place_guard}.rs` | same |
| Progress modal | `resonance-app/tests/io/import_progress_dialog.rs`; goldens `import_progress_modal_{preparing,in_progress,complete}-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test io` |
| MIDI import | `resonance-app/tests/io/import_dialog*.rs`, `import_entry_points.rs`, `control/control_import_midi.rs`, `resonance-audio/tests/midi_hw/smf_import.rs`; goldens `import_dialog_{drop,review,tempo_conflict}-wgpu.png` | same |

**Not covered:**

- The real `rfd` file picker.
- A real OS drag of a file onto the window (tests send `WindowAudioDrop`
  directly).
- Importing very large or long files.
- The disk side effects of undoing an import: the orphaned
  `asset_<id>.wav` stays on disk.

## Decisions and deviations

- **Clips are copied, not shared.** A clip placed from an asset first plays
  `audio/asset_<id>.wav`. On save, though, the engine writes every clip to
  its own `audio/clip_<id>.wav` (`save_clips_to_project_dir`), and after a
  reload the clip plays that copy. The pool is therefore an index plus
  provenance, not shared storage, and a sample placed N times is stored
  N + 1 times. The design wanted clips to reference pool assets. See
  [intent](../../intents/timeline/pool-backed-clip-audio.md).
- Dropping a file from the OS always makes a new track at the playhead. The
  design's "drop on a lane" applies only to the browser drag.
- The pool-only import has no GUI way to then place the asset. See
  [media-browser.md](../ui-shell/media-browser-panel.md).
- An import with an undefined project folder used to be dropped silently. It
  is now refused with a banner.

## Known gaps

- Pool assets have no thumbnail peaks after a reload. Nothing rebuilds them,
  despite the comment in `restore_pool_assets`, so Pool rows draw a flat
  baseline until the asset is relinked.
- Undoing an import leaves `asset_<id>.wav` on disk. It is harmless, and the
  id allocator skips past it.
- Pool assets can't be placed or auditioned from the GUI:
  [browser-drag-to-timeline](../../intents/ui-shell/media-browser-drag-and-pool-audition.md).
- Storage is duplicated per clip:
  [pool-backed-clip-audio](../../intents/timeline/pool-backed-clip-audio.md).
- MIDI import never sets the Vocal kind on its own, and it ignores the
  file's time signatures.
