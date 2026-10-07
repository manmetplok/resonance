# Missing-media relink

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/intents/prototypes/audio-sample-import-media-browser/DESIGN.md` (doc #175, todos #600 / #607; W4 / FU-A1a3 for clips).

A project can be opened without its audio, for example after it was moved
without its `audio/` folder or opened on another machine. The clips stay in
the timeline but play nothing, and the app offers to find the files again.
Users locate files one at a time, or point at a folder and let the app
resolve every missing file by name. A relinked file is copied back into the
project, so the project is self-contained again.

## Behaviour

- **Two kinds of missing media**, both detected at load or restore:
  - *Pool assets*: `restore_pool_assets` flags `PoolAsset::missing` when
    `audio/asset_<id>.wav` is absent. The asset is kept, not dropped.
  - *Clips*: `reconcile::clips::load_audio_clip` adds the clip id to
    `RelinkState::missing_clips` when the clip's own `audio/clip_<id>.wav` is
    absent. The clip stays on the timeline and is silent. This set is
    worked out again on every load and restore and is never saved.
- **The modal opens by itself** after a project **load** that has any missing
  asset or clip (`engine_events/project_io.rs`). Undo and redo never open it.
  Users can also open it from a Pool row's `relink` chip or the palette's
  "Relink Missing Media…". When it opens it takes a snapshot of the missing
  set, so rows that get resolved stay listed and show "Relinked", and the
  footer counts "N of M relinked".
- **Per-file Locate…** opens a file picker limited to wav/flac/mp3/ogg.
  - *For an asset*, the chosen file is imported again with
    `resonance_audio::import_one_to_pool` under the **same** id, as
    `audio/asset_<id>.wav`, on a blocking worker. When it finishes
    (`RelinkMessage::Imported`), `apply_relinked_asset` refreshes the
    asset's metadata, clears `missing`, and sends `LoadClipFromWav` again
    for every clip whose `asset_ref` points at the asset, keeping each
    clip's own trims and position.
  - *For a clip*, the file becomes a **new** pool asset with a fresh id. The
    clip is pointed at it and reloaded. Trims that no longer fit the new
    file's length are reset to zero.
- **Search a folder…** walks the chosen folder breadth-first on a blocking
  worker (`scan_folder_for_names_with`).
  - Depth is capped at 24. Unreadable folders are skipped. Entries are
    visited in name order, so the shallowest match wins and the result is
    deterministic.
  - A file matches when its filename equals the asset's original filename,
    ignoring case.
  - The walk can be cancelled. The modal shows how many folders have been
    scanned, and a result from a cancelled or replaced walk is dropped.
  - When the walk ends, one relink import starts for each asset that is
    still missing and was found. Only one walk runs at a time.
  - If nothing was found, the error "No missing files were found in …" is
    shown.
- **Guards:**
  - A project folder is required.
  - An asset that is already being relinked, or is no longer missing, is
    ignored.
  - If the asset or clip was deleted while its import ran, the result is
    dropped and the new WAV is simply left on disk.
  - Only the last failure is kept (`RelinkState::last_error`).
- **Undo:** only a successful result records an entry (`Imported(Ok)` /
  `ClipImported(_, Ok)` are `Record`). On replay, `missing` is worked out
  from the disk again. Undoing an asset relink therefore restores the old
  source path and metadata but does not hide the restored file again.
- **Leave offline** closes the modal and cancels any running folder walk.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Session state | `resonance-app/src/state/relink.rs` | `RelinkState` (`in_flight`, `missing_clips`, `clips_in_flight`, `modal_targets`, `scan`), `ScanControl`, `RelinkScan` |
| Durable flag | `resonance-app/src/state/pool.rs` | `PoolAsset::missing`, `MediaPool::missing_assets` |
| Handlers | `resonance-app/src/update/relink.rs` | `RelinkMessage`, `start_relink`, `start_clip_relink`, `start_batch_relink`, `finish_batch_relink`, `apply_relinked_asset`, `scan_folder_for_names_with` |
| Detection | `resonance-app/src/update/project_io/replay/restore.rs`, `update/project_io/reconcile/clips.rs` | `restore_pool_assets`, `load_audio_clip` |
| Auto-open | `resonance-app/src/engine_events/project_io.rs` | `open_relink_modal` after load |
| Engine import step | `resonance-audio/src/engine/import_pool.rs` | `import_one_to_pool` (called directly, not through the engine thread) |
| View | `resonance-app/src/view/relink_dialog.rs` | `view_relink_dialog_overlay`, `relink_row`, `relink_chip` |

Relink imports run as app tasks (`Task::perform` + `spawn_blocking`). They do
not go through `AudioCommand::ImportAudioToPool`, so they never show in the
import-progress modal.

## Surfaces

- **GUI:** the relink modal (auto-opened on load); the `relink` chip on
  missing Pool rows in the media browser; the command palette's
  `RelinkMissingMedia`.
- **Control API / MCP:** none. `pool.list` reports `missing: true`, and the
  tool descriptions tell agents that relinking happens in the GUI.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Asset relink, metadata, clip reload, undo | `resonance-app/tests/io/relink.rs` | `./scripts/run-tests.py -p resonance-app` |
| Clip relink (W4) | `resonance-app/tests/io/relink_clip.rs` | same |
| Folder walk, cancel, stale result | `resonance-app/tests/io/relink_folder_scan.rs` | same |
| Modal behaviour + goldens | `resonance-app/tests/io/relink_modal.rs`; `relink_modal_{two_missing_files,one_resolved,searching,missing_clip}-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test io` |

**Not covered:**

- The real OS file and folder pickers.
- A walk over a very large tree.
- A symlink loop (only the depth cap guards against one).

## Decisions and deviations

- The design's relink chip and modal shipped as designed.
- Relinking missing *clips*, not just assets, was added later (W4). Projects
  copy every clip's audio to `audio/clip_<id>.wav` on save (see
  [audio-pool-and-import.md](./audio-pool-and-import.md)), so a missing clip
  file is the more common case after a move.
- A relinked file is copied into the project rather than referenced where it
  was found. This keeps the project self-contained.

## Known gaps

- "Search a folder…" resolves only pool assets. Missing clips have no
  original filename to search for (their files are named `clip_<id>.wav`),
  so each one has to be located by hand.
- No control method exists, so an agent can see that media is missing but
  can't fix it.
- Relink imports do not show per-file progress beyond the "Relinking…"
  label on each row.
