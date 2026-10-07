# Audio file decoding

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none in `docs/history/specs/`. Leads: code review LIB-01 (resampler), DEP-06 (plugin codec registry), ba todo #1260 (mmap clips).

How audio files on disk become audio the engine can play: decoding source
files of several formats into stereo at the project rate, and reading the
engine's own WAVs back without a copy. Importing into the pool, relinking
and the media browser are timeline features
([../timeline/audio-pool-and-import.md](../timeline/audio-pool-and-import.md));
this doc covers the decode layer they share.

## Behaviour

### Decoding source files

- `resonance_common::decode_file(path, target_rate)` (behind the `decode`
  feature) probes the file with symphonia 0.6 and decodes it to interleaved
  f32. Compiled-in formats: WAV/PCM, FLAC, MP3, Ogg Vorbis, AAC and MP4
  (`isomp4`).
- **Channels:** stereo is kept; mono is copied to both sides; files with
  more than two channels keep only the first two (no downmix).
- **Rate:** anything more than 1 Hz from the target is resampled with the
  band-limited windowed-sinc resampler (`linear_resample_stereo`; the name
  is historical, it has not been linear since LIB-01).
- **Which files reach it:** the app's file pickers and window drops accept
  `wav`, `flac`, `mp3`, `ogg` only (`POOL_AUDIO_EXTENSIONS`, and the same
  lists for relink and the reference track). `pool.import` over the control
  API checks only that each path is an absolute existing file, so any format
  symphonia can probe (including AAC/M4A) decodes there (code reading, not
  tested).
- Pool imports write the result as `audio/asset_<id>.wav`, stereo 32-bit
  float at the project rate (`engine/import_pool.rs`); a decoder panic fails
  only that file.
- `probe_audio_file` reads format, channels, rate and length without
  decoding, for the media browser and pool metadata (`AudioFormat`:
  Wav, Flac, Mp3, Ogg, Aac, Mp4, Other).

### Reading engine WAVs

- Recorded takes and pool assets are stereo 32-bit float WAVs.
  `io::wav::map_wav_file` memory-maps one and
  `locate_wav_float_data` walks the RIFF chunks to the PCM data; the mixer
  then reads `&[f32]` straight from the page cache. A WAV in any other
  layout is refused with a parse error (`WavParseError`).
- A WAV whose fmt-chunk rate differs from the engine rate (a project made
  at another PipeWire rate) is resampled once at load into memory
  (`ClipSource::open_wav_at_rate`) and written back at the engine rate on the
  next save. Clip *positions* are not rescaled; see
  [project-sample-rate-mismatch](../../intents/hardware-io/project-sample-rate-mismatch.md).

### Plugins

Plugins that load samples or impulse responses (drums, IR) decode through
`decode_wav_*` with a smaller symphonia registry that has only WAV and FLAC
(`plugin_probe` / `plugin_codecs`), so the plugin binaries do not link the
MP3/AAC/Vorbis decoders (DEP-06).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Decode + resample | `resonance-common/src/wav.rs`, `resonance-common/src/resample.rs` | `decode_file`, `decode_wav_stereo`, `decode_wav_split`, `to_stereo_interleaved`, `linear_resample_stereo`, `StreamingLinearResampler`, `plugin_probe` |
| Probe | `resonance-common/src/audio_probe.rs` | `probe_audio_file`, `AudioFormat`, `AudioInfo` |
| Engine re-export | `resonance-audio/src/decode.rs` | `decode_file`, `linear_resample` |
| mmap reader | `resonance-audio/src/io/wav.rs` | `map_wav_file`, `locate_wav_float_data`, `MappedWav` |
| Clip source | `resonance-audio/src/types/clip.rs` | `ClipSource::open_wav`, `open_wav_at_rate` |
| Pool import worker | `resonance-audio/src/engine/import_pool.rs` | `import_one_to_pool` |

The only `unsafe` for mapping lives in `map_wav_file`.

## Surfaces

- **GUI:** "Import audio…", drop on the window, media browser, relink,
  reference track (all timeline / mixer features).
- **Control API / MCP:** `pool.import` → `mcp__resonance__pool_import`;
  `reference.load` → `mcp__resonance__reference_load`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Decode, channel split, probe | `resonance-common/tests/wav.rs`, `wav_split.rs`, `audio_probe.rs` | `./scripts/run-tests.py -p resonance-common` |
| Resampler quality, streaming, no allocation | `resonance-common/tests/resample_quality.rs`, `streaming_resample.rs`, `resample_alloc.rs` | same |
| RIFF chunk walk | `resonance-audio/tests/io/wav_chunk_parse.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Rate-mismatch load | `resonance-audio/tests/io/load_wav_rate_mismatch.rs` | same |
| Pool import worker | `resonance-audio/tests/io/import_audio_to_pool.rs` | same |

Not covered: AAC/MP4 decoding and the control path's lack of an extension
filter.

## Decisions and deviations

- Multichannel sources are cut to their first two channels rather than
  folded down.
- One decoder for the app, a WAV+FLAC-only registry for plugins (DEP-06).

## Known gaps

- Clip positions are not rescaled when the engine rate differs from the
  project's: [project-sample-rate-mismatch](../../intents/hardware-io/project-sample-rate-mismatch.md).
