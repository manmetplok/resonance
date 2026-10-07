# Offline render, mixdown export and bounce in place

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: "doc #196" (export pipeline, todos #650–#655) is cited in
> code; prototype `docs/intents/prototypes/compressed-loudness-export-formats/`;
> code-review findings ENG-06, ENG-07, ENG-08, ENG-13, MIX-02.

The engine can render the project faster than realtime: the whole mix to a
file (with optional loudness normalisation), or one instrument track to a
new audio clip ("bounce in place"). Every offline renderer shares one chunked
render core with the live mixer, so a file sounds like playback. Stems,
measurement and freeze use the same core and have their own pages:
[stem-export.md](stem-export.md), [offline-measurement.md](offline-measurement.md),
[track-freeze.md](track-freeze.md). The export dialog and bounce menus are in
[overlays-and-dialogs.md](../ui-shell/overlays-and-dialogs.md).

## Behaviour

### Common to every offline render

- Runs on its own worker thread under `run_supervised`, holding an
  `OfflineRenderGuard` taken on the engine thread before the spawn. While it
  is held the live callback outputs silence and Play / Record refuse
  ([rt-safety.md](rt-safety.md)).
- Refuses to start while the transport rolls (`TransportRunning`).
- Tells every plugin it renders offline (`OfflineRenderMode`, CLAP
  `render.set`), resets plugins at the start so output is deterministic, and
  restores realtime mode on every exit.
- Rebuilds vocal-tuning caches first, so retuned clips render corrected.
- `render_chunk` renders `BOUNCE_CHUNK`-sized chunks through
  `mixer::render_block` with `RenderStrategy::Bounce` and its own render pool
  (normal priority). Tail chunks are padded to the CLAP minimum
  (`MIN_CLAP_FRAMES`) and trimmed.
- Plugin-parameter automation is baked (the automation snapshot is passed
  in). Offline renders see settled bypass states only.
- PDC: the render builds its own latency table, renders `max_latency` extra
  frames and drops that many from the front.
- Cancel tokens are per render and checked between chunks.
- Output is written to `<target>.partial` and renamed over the target only
  when complete (`PartialFile`), so a failed or cancelled render never
  damages the file it would replace.
- The reference A/B monitor is never in an offline render.

### Mixdown export

- Range: earliest clip start (audio or MIDI) to the latest clip end, plus a
  2 s FX tail (`BOUNCE_TAIL_SECONDS`). Offset 0 in the file is the earliest
  clip, not bar 1. An empty project fails with `NoAudio`.
- Includes master FX and master volume (master-gain automation ramps per
  chunk at the comp-delayed position), then a hard clip, unless normalising.
- `ExportAudio { path, settings: ExportSettings }`:
  - `format`: `Wav { bit_depth: I16 | I24 | F32, sample_rate }`,
    `Flac { bit_depth, sample_rate, compression: Fast | Default | Max }`
    (pure-Rust `flacenc`), `Mp3 { mode: Cbr | Vbr, bitrate_kbps }`
    (libmp3lame, `mp3` feature), `Opus { bitrate_kbps, optimize }`
    (libopus + Ogg, `opus` feature, always 48 kHz). A format whose encoder
    is not compiled in fails with `EncoderUnavailable` before any file is
    created.
  - Export-time resampling: a `rubato` FFT stage when the WAV/FLAC
    `sample_rate` differs from the engine rate; output length is exact.
  - `normalize: NormalizeSpec { enabled, mode: IntegratedLufs | TruePeak,
    target_db, ceiling_dbtp }` (default off, -14 LUFS, -1 dBTP). When on:
    pass 1 renders and measures (BS.1770-4 integrated LUFS and 4×
    oversampled true peak), pass 2 renders again through a true-peak
    lookahead limiter that applies the gain trim, re-measures, and reports
    the achieved figures. No hard clip in this mode.
  - `metadata` (title, artist, album, year) is written only by the Opus sink.
  - Events: `ExportProgress { phase: Analyze | Render | Encode, fraction }`,
    then `ExportComplete { path, achieved_lufs, achieved_dbtp, bytes }` or
    `ExportError { kind, message }`.
- `BounceToWav { path }` is the legacy shim: `ExportAudio` with 32-bit float
  WAV at the engine rate, reporting through `BounceProgress` /
  `BounceComplete` / `BounceError`.
- `CancelBounce` cancels a running export or bounce.

### Bounce in place

- Offline (`BounceTrackToAudio { source_track_id, target_track_id,
  target_clip_id, name }`): the app first creates the audio target track.
  The worker renders the source and its sub-tracks over the source's MIDI
  extent (or the loop range when the loop is on) plus 2 s, without master FX
  or volume (the clip plays through master later), into an in-RAM clip. The
  engine thread adds the clip and sends `TrackBounceCompleted`; the source
  is muted. Cancel removes the target track (`BounceTargetCancelled` →
  `TrackRemoved` + `TrackBounceCancelled`).
- Realtime (`BounceTrackRealtimeToAudio`) for tracks that drive an external
  MIDI device: mutes every other track, arms the target with the given input,
  plays from the first MIDI start, records the hardware return, stops at the
  end + 2 s and restores the mutes. See [external-instruments.md](../hardware-io/external-instruments.md).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Spawns, guards, partial files | `resonance-audio/src/engine/bounce/mod.rs` | `export_spawn`, `to_audio_clip_spawn`, `OfflineRenderGuard`, `CallbackActivity`, `PartialFile` |
| Chunk core | `resonance-audio/src/engine/bounce/render.rs` | `render_chunk`, `ChunkCtx`, `ChunkScratch`, `chunk_span`, `BOUNCE_CHUNK`, `MIN_CLAP_FRAMES`, `OfflineRenderMode`, `reset_plugins`, `build_latency_comp`, `try_lock_with_backoff` |
| Export driver | `resonance-audio/src/engine/bounce/wav.rs` | `run_export`, `ExportReporter`, `output_sample_rate` |
| Encoders | `resonance-audio/src/engine/bounce/encoder.rs` | `EncoderSink`, `WavSink`, `FlacSink`, `Mp3Sink`, `OpusSink`, `build_sink` |
| Normalise + limiter | `resonance-audio/src/engine/bounce/normalize.rs`, `limiter.rs` | `LoudnessMeasure`, `target_gain_db`, `TruePeakLimiter` |
| Export resampling | `resonance-audio/src/engine/bounce/resample.rs` | `ResampleStage` |
| Bounce in place | `resonance-audio/src/engine/bounce/clip.rs`, `engine/bounce_realtime.rs`, `engine/bounce_common.rs` | `to_audio_clip`, `apply_bounced_clip`, `PendingBounce`, `poll_pending_bounce`, `midi_render_range`, `BOUNCE_TAIL_SECONDS` |
| Settings types | `resonance-audio/src/types/export.rs` | `ExportSettings`, `ExportFormat`, `NormalizeSpec`, `ExportMetadata` |
| Dispatch | `resonance-audio/src/engine/thread/dispatch/bounce.rs` | `dispatch_bounce` |
| App: control mixdown | `resonance-app/src/update/control/render.rs` | `normalize_request`, `start_normalized_export` |

## Surfaces

- **GUI:** the "Bounce to WAV" command (Cmd/Ctrl+B, command palette, master
  inspector button) opens a save dialog → `BounceToWav` with a progress
  modal; "Bounce Selected Track in Place" (command palette, mixer track inspector) opens
  the bounce-in-place dialog. No GUI path sends `ExportAudio`, so there
  is no format, sample-rate, normalisation or metadata choice in the GUI.
- **Control API / MCP:** `render.mixdown` → `mcp__resonance__render_mixdown`.
  A path ending in `.wav`; `overwrite`; optional `platform` (Spotify, Apple,
  YouTube, Tidal, Amazon, Deezer, Club) or `normalize { target_lufs,
  ceiling_dbtp }` → `ExportAudio` with normalisation; otherwise
  `BounceToWav`. Only the whole song (a partial `range` is rejected). Runs
  as a job (`job_status`, `job_wait`).
- **Agent skills:** `mastering` (delivery, `render_mixdown` with a
  platform).

## How it's tested

| What | Where | Run |
|---|---|---|
| Fidelity vs playback | `resonance-audio/tests/bounce/offline_render_fidelity.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Encoders, settings, normalise | `tests/bounce/export_encoders.rs`, `export_settings.rs`, `export_normalize.rs`, `delivery_normalize.rs` | same |
| Range, tail, tempo, MIDI, automation | `tests/bounce/bounce_render_range_tempo.rs`, `bounce_tail_and_master_latency.rs`, `bounce_midi_events.rs`, `automation_render.rs` | same |
| Transport guard, plugin lock, render mode | `tests/bounce/bounce_transport_guard.rs`, `bounce_plugin_lock.rs`, `offline_render_mode.rs` | same |
| External offsets, reference exclusion, vocal tuning | `tests/bounce/bounce_external_offsets.rs`, `reference_export_exclusion.rs`, `vocal_tuning_bounce.rs` | same |
| App + control | `resonance-app/tests/control/control_render.rs`, `control_render_normalize.rs`, `control_render_wav_geometry.rs`; `tests/io/mixdown_progress_modal.rs`, `offline_render_gate.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: MP3 and Opus output decoded back and compared (the encoder
tests check structure and sizes); bounce in place end to end through the
app with a real instrument.

## Decisions and deviations

- The prototype's format and loudness choices exist engine-side, but the
  only caller of `ExportAudio` is the control layer's normalised WAV.
- A normalised export skips the hard clip and lets the limiter set the
  ceiling (ENG-06).
- `TruePeak` normalise mode trims by measured dBTP; the control API only
  uses `IntegratedLufs`.

## Known gaps

- FLAC/MP3/Opus, export sample rate and metadata have no GUI or control
  surface, and WAV/FLAC/MP3 drop the metadata:
  [export-formats-and-metadata](../../intents/audio-engine/export-formats-and-metadata.md).
- `render.mixdown` cannot render a range:
  [render-range-and-stems-control](../../intents/control-api/render-range-and-stems.md).
