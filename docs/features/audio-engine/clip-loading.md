# Clip loading, import and persistence (engine side)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: prototype `docs/intents/prototypes/audio-sample-import-media-browser/`;
> code-review findings STATE-08, STATE2-02, FU-A13e, FU-D7c, FU-V5b, ARCH-02
> B-5 and "ba doc #276" cited in code.

How audio files become clips the mixer can play: importing a file into the
project's media pool, loading a clip's WAV when a project opens, and writing
clip WAVs into the project on save. Clip placement and editing (move, trim,
split, fades, gain) are in [clip-editing.md](../timeline/clip-editing.md); the pool UI and
relink are in [audio-pool-and-import.md](../timeline/audio-pool-and-import.md) and [relink.md](../timeline/relink.md).

## Behaviour

### Playback format

The engine plays **32-bit float stereo** WAVs at the engine rate.
`ClipSource` is either:
- `Mapped`: the file memory-mapped and sliced to its PCM data chunk; the
  mixer reads `&[f32]` straight from the page cache. Mapping pre-touches
  every page off the audio thread. This is what every file the app writes
  becomes.
- `Memory(Arc<[f32]>)`: in-RAM samples, for freshly decoded audio not yet
  written, for a WAV whose data chunk is not 4-byte aligned (some DAWs write
  odd-length `LIST` chunks; casting would panic on the audio thread), and for
  a WAV at another rate.

A clip copy (copy-on-write in the render graph) shares the samples; nothing
is duplicated.

### Import to pool (`ImportAudioToPool { files }`)

- Every file is reported `ImportProgress { Queued }` up front, then
  `Working`, then `Done` after its `AssetImported`; a failure ends with
  `ImportFailed` instead.
- Per file (`import_one_to_pool`, pure): probe source metadata, decode
  WAV/FLAC/MP3/OGG through `resonance_common::decode_file` (symphonia),
  up/down-mix to stereo, resample to the engine rate with the shared
  band-limited resampler, write `{project_dir}/audio/asset_{id}.wav`
  (32-bit float, created with `O_EXCL`; an existing file is never
  overwritten), compute waveform peaks.
- The app allocates asset ids; the engine does not keep pool assets, it only
  writes the file and reports metadata. Placing a clip is a later step.
- Runs on the bounded import queue. A `ClearAll` during an import fences
  it: results from the old project are dropped.

### Loading a clip (`LoadClipFromWav`, `LoadTakeClipFromWav`)

- Sent per audio clip on project load. The engine thread issues a **load
  ticket**, then a worker maps the file (`ClipSource::open_wav_at_rate`),
  computes peaks and posts `EngineInternal::ClipLoaded` to the engine inbox.
- The engine thread applies it only if the ticket is still current (a later
  load supersedes it; a `DeleteClip` withdraws it), the `ClearAll`
  generation matches, and the id is not already present; then it publishes
  the clip and sends `ClipImported` (timeline) or nothing (restored take).
- Edits for a clip still loading (move, trim, fade, …) are parked as
  `DeferredClipCommand`s and replayed when it lands, or dropped after 10 s.
- A WAV at a different rate (project recorded at 44.1 kHz, engine at 48 kHz)
  is resampled to the engine rate in RAM; the next save re-encodes it.
- `ImportQueue` runs at most `MAX_CONCURRENT_IMPORTS` (4) workers; requests
  beyond that queue (unbounded), they are never dropped.

### Writing clip WAVs

- `SaveClipsToProjectDir` (project save): every clip gets
  `{project_dir}/audio/clip_{id}.wav` (copied if it maps a file elsewhere,
  encoded if in RAM). Always answers exactly once:
  `ClipsSavedToProjectDir { clip_files }` or `ClipsSaveFailed { error }`.
- `PersistClipWavs` (sent whenever the app captures an undo snapshot):
  silently writes a WAV for every clip that has none, so a full-reload undo
  finds the audio.
- `SetProjectDir` starts a background scan that raises the engine's own clip
  counter past existing `clip_{id}.wav` files (STATE-08). Since recordings
  draw ids from the app's grant (D-7d) nothing allocates from that counter;
  the scan is kept until D-7f deletes it. What protects an existing WAV now
  is the app's allocator and the grant skipping ids whose WAV exists.
- `transcode_to_wav` (overwrite) and `transcode_to_wav_new` (refuse if
  present) are the shared writers.

### Other per-clip analysis

`DetectClipTempo` and `AnalyzeClipPitch` run on workers and post back the
same way (see [warp-and-tempo-detection.md](../timeline/warp-and-tempo-detection.md) and [vocal-rendering.md](../compose/vocal-rendering.md) for
the features).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Clip model | `resonance-audio/src/types/clip.rs` | `AudioClip`, `ClipSource::{Mapped, Memory}`, `open_wav`, `open_wav_at_rate`, `compute_waveform_peaks`, `ClipError` |
| WAV mapping + chunk walk | `resonance-audio/src/io/wav.rs` | `map_wav_file`, `locate_wav_float_data`, `WavParseError`, `WavIoError` |
| Decode / resample | `resonance-audio/src/decode.rs` → `resonance-common` | `decode_file`, `linear_resample` (band-limited, name historical), `StreamingLinearResampler` |
| Load, defer, save, persist | `resonance-audio/src/engine/clips.rs` | `handle_load_clip_from_wav`, `submit_clip_load`, `apply_clip_loaded`, `poll_deferred_clip_commands`, `handle_save_clips_to_project_dir`, `handle_persist_clip_wavs`, `transcode_to_wav`, `transcode_to_wav_new`, `start_clip_id_scan` |
| Tickets | `resonance-audio/src/engine/clip_loads.rs` | `ClipLoadTickets` |
| Worker pool | `resonance-audio/src/engine/import_queue.rs` | `ImportQueue`, `MAX_CONCURRENT_IMPORTS` |
| Pool import | `resonance-audio/src/engine/import_pool.rs` | `import_one_to_pool`, `run_pool_import`, `PoolImportOutcome`, `POOL_IMPORT_CANCELLED` |
| Inbox | `resonance-audio/src/engine/internal.rs` | `EngineInternal::{ClipLoaded, ClipLoadFailed}` |

## Surfaces

- **GUI:** media browser and import dialog, pool tab, drag to timeline,
  relink ([media-browser-panel.md](../ui-shell/media-browser-panel.md), [audio-pool-and-import.md](../timeline/audio-pool-and-import.md), [relink.md](../timeline/relink.md)).
- **Control API / MCP:** `pool.import` → `mcp__resonance__pool_import`,
  `pool.list` → `mcp__resonance__pool_list`, `pool.remove_unused` →
  `mcp__resonance__pool_remove_unused`, `clip.place` →
  `mcp__resonance__clip_place`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Off-thread load, stale after `ClearAll`, rate mismatch, chunk parse | `resonance-audio/tests/io/load_clip_offthread.rs`, `load_clip_stale_after_clear_all.rs`, `load_wav_rate_mismatch.rs`, `wav_chunk_parse.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Pool import | `tests/io/import_audio_to_pool.rs` | same |
| Delete cancels load, deferred commands, save always answers, persist | `tests/engine/clip_delete_cancels_load.rs`, `deferred_clip_commands.rs`, `save_clips_always_answers.rs`, `persist_clip_wavs.rs` | same |
| App | `resonance-app/tests/io/import_*.rs`, `pool_*.rs`, `relink*.rs`, `clip_wav_gc.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: very large files under memory pressure (page-cache eviction of
mapped clips during playback).

## Decisions and deviations

- Clips stream from mmapped files instead of being decoded into RAM, so a
  long project costs page cache, not heap.
- Loads queue instead of being refused past the worker cap (a refusal used
  to drop clips silently from large projects).

## Known gaps

- Warp settings on a clip are stored but never rendered:
  [clip-warp-render](../../intents/audio-engine/clip-warp-render.md).
