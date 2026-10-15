# Clip storage and clip WAV GC

> Verified against master @ `acac5ea3` on 2026-10-15.
> Spec history: `docs/history/reviews/code-review-todo.md` (FU-V5a, FU-V5b,
> STATE-08, STATE-12, UX-03); take reader from epic #15 / ba todo #1400.

Every audio clip's sound lives in one file, `audio/clip_<id>.wav`, inside the
project bundle. This doc covers how those files come to exist, how an undo
snapshot or a take finds them again, and how a manual save deletes the ones
nothing can name any more. The pool *UI* (Files/Pool tabs, relink, import
dialogs) is in the timeline area ([audio-pool-and-import.md](../timeline/audio-pool-and-import.md), [relink.md](../timeline/relink.md)) and [media-browser-panel.md](../ui-shell/media-browser-panel.md).

## Behaviour

- **One file per clip id.** `clip_audio_file(id)` = `audio/clip_<id>.wav`.
  `ProjectClip.audio_file` stores that path; a take names its recording only
  by `clip_ref`. Pool imports are separate files, `audio/asset_<id>.wav`.
- **Written once.** The engine streams recordings straight into the file,
  writes renders and bounces once, and never rewrites a clip WAV. Clip ids
  are never reused in a session (STATE-08), so a file name always means the
  same audio. Loads memory-map the file (`ClipSource::Mapped`).
- **Persisted before undo needs it.** `snapshot_for_undo` sends
  `PersistClipWavs` whenever the project has audio clips, so the engine
  writes a WAV for any in-engine clip that has none yet, before the edit's
  own commands run. An undo that re-adds a deleted clip then finds its audio.
- **Untitled projects** use the session's scratch dir
  (`<data>/resonance/autosave/<session-id>/`) as the engine's project dir, so
  this all works before the first save; the first Save As copies the WAVs the
  undo history still names into the bundle (`carry_clip_wavs`).
- **Save As carries the history's clips.** The engine writes only the live
  clips into a new bundle, and an undo resolves a snapshot's `audio/clip_<id>.wav`
  against the *current* bundle. So a Save As remembers the bundle it left
  (`ProjectIoState::carry_clip_wavs_from`), and the first save to the new
  one that reaches its write copies, from the old `audio/`, every WAV the
  project or an undo/redo snapshot names (`Resonance::history_clip_ids`)
  that the new one lacks (BUG-60). Both carries run in the save's write
  task, off the update loop, before the GC; an existing file is never
  overwritten, and a source that is the target (also through a symlink,
  `same_dir`) copies nothing. A save that fails before its write keeps the
  source for the retry; opening or creating another project drops it.
- **Take waveforms are read from disk.** A cycle-record take is never a
  mirrored clip, so `load_take_peaks` reads its WAV to build the lane's
  peaks and missing-media state. It runs on `TakeCaptured`, on project load
  and on every undo/redo (the `TakeGroups` reconcile domain); an
  already-cached recording is skipped, because the recording is immutable.

### Save-time GC

After a successful **manual** save (never an autosave),
`reap_unreferenced_clip_wavs` deletes `audio/clip_<id>.wav` files that
nothing names. It keeps a file when its id is named by:

- the project just saved, any undo or redo snapshot, the app's clip mirror,
  or the engine's clip list at save time (`Resonance::clip_gc_keep`);
- any `*.json` in the bundle root (the autosave) or in `backups/` — scanned
  for any string naming a `clip_<id>.wav` and any `clip_ref` number.

It also keeps every id above the highest named one (a clip created after the
keep set was taken). It aborts the whole pass if one JSON cannot be read or
parsed, keeps everything if nothing names any clip, and only touches direct
children of `audio/` named exactly `clip_<digits>.wav`. GC is skipped while
recording. Rendered vocal takes have their own reaper
(`reap_orphaned_vocal_takes`, skipped while a render is in flight).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| GC + carry | `resonance-app/src/project/clip_gc.rs` | `reap_unreferenced_clip_wavs`, `carry_clip_wavs`, `same_dir`, `collect_clip_ids`, `clip_wav_id` |
| Keep set, carries + call site | `resonance-app/src/engine_events/project_io.rs` | `Resonance::clip_gc_keep`, `Resonance::history_clip_ids`, `clip_wav_carries`, `try_finish_save` |
| Take reader | `resonance-app/src/project/take_audio.rs` | `load_take_peaks` |
| Persist before snapshot | `resonance-app/src/undo/snapshot.rs` | `snapshot_for_undo` (`AudioCommand::PersistClipWavs`) |
| Untitled anchor, Save As source | `resonance-app/src/update/project_io/{mod.rs,save_as.rs,recovery.rs}` | `anchor_untitled_project`, `undo_anchor_dir`, `save_as_to` (sets `carry_clip_wavs_from`), `after_save` (removes the scratch dir) |
| Engine side | `resonance-audio/src/engine/clips.rs`, `types/commands.rs` | `SaveClipsToProjectDir`, `PersistClipWavs`, `LoadClipFromWav` |
| Vocal takes | `resonance-app/src/update/compose/vocal_audio_io.rs` | `reap_orphaned_takes`, `vocal_audio_dir` |

Invariant: a clip id that ever named a WAV must never be issued again in the
session. The GC's "keep anything newer than the newest named id" rule and the
undo history both depend on it. See [entity-ids.md](entity-ids.md).

## Surfaces

- **GUI:** none of its own; it runs inside Save.
- **Control API / MCP:** none of its own. `pool.remove_unused`
  (`mcp__resonance__pool_remove_unused`) removes unused pool *assets*, which
  is a different file family (timeline area).

## How it's tested

| What | Where | Run |
|---|---|---|
| GC keep rules, carry rules | `resonance-app/tests/io/clip_wav_gc.rs` | `./scripts/run-tests.py -p resonance-app` |
| Save As carries undo-only clips; undo finds them | `resonance-app/tests/io/save_as_history_clips.rs` | same |
| Undo finds re-added clip audio | `resonance-app/tests/timeline/undo_clip_audio_persist.rs` | same |
| Take lanes + peaks across load | `resonance-app/tests/io/take_lanes_persistence.rs` | same |
| Untitled anchor / carry-over | `resonance-app/tests/io/project_lifecycle_ux.rs` | same |
| Engine writes missing WAVs | `resonance-audio/tests/engine/persist_clip_wavs.rs` | `./scripts/run-tests.py -p resonance-audio` |

Not covered: app and engine together — app tests assert only that
`PersistClipWavs` is sent, engine tests only what it writes.

## Decisions and deviations

- The GC is deliberately conservative: a backup shares the bundle's audio by
  name, so backups and the autosave count as references.
- Take peaks are derived from the file rather than sent on `TakeCaptured`,
  because a project load and an undo have no event to carry them.

## Known gaps

- Orphaned `asset_<id>.wav` files left by an undone import are never
  deleted; they only raise the asset-id counter on reopen.
- A clip placed from the pool gets its own `clip_<id>.wav` copy at the first
  save. See [pool-backed-clip-audio](../../intents/timeline/pool-backed-clip-audio.md).
