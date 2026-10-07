# Expose export formats, sample rate, normalisation and metadata

> Status: **partly built** (engine done, no GUI or control surface)
> Area: audio-engine (`docs/features/audio-engine/offline-render-and-export.md`)
> with the export dialog in project-core
> Source: prototype `docs/intents/prototypes/compressed-loudness-export-formats/`;
> confirmed on master @ `8eac71b3`.

## Why

The engine can write WAV (16/24/32f), FLAC, MP3 and Opus, resample on
export, and loudness-normalise with a true-peak limiter. A user cannot reach
any of it: the GUI only sends `BounceToWav` (32-bit float WAV at the engine
rate), and the control API only uses `ExportAudio` for a normalised 32-bit
float WAV. Delivering an MP3 for review or a 44.1 kHz / 16-bit master needs
an external tool. Metadata (title, artist, album, year) is accepted by the
command but only the Opus sink writes it.

## What it would do

- The export dialog offers format, bit depth, sample rate, normalisation
  (target LUFS or a platform preset, ceiling) and metadata, and reports the
  achieved loudness and file size.
- `render.mixdown` accepts a format (and the path extension matches it).
- WAV (`LIST/INFO`), FLAC (Vorbis comments) and MP3 (ID3) write metadata.

## What exists to build on

- `resonance-audio/src/types/export.rs` (`ExportSettings`, `ExportFormat`,
  `NormalizeSpec`, `ExportMetadata`).
- `engine/bounce/{wav,encoder,normalize,limiter,resample}.rs`;
  `ExportProgress` / `ExportComplete { achieved_lufs, achieved_dbtp, bytes }`
  / `ExportError { kind }` events (`EncoderUnavailable` for a format not
  compiled in).
- `resonance-app/src/update/control/render.rs` (`start_normalized_export`,
  platform presets in `resonance-control/src/methods/render.rs::Platform`).
- Export dialog (`resonance-app/src/view/export_dialog.rs`) already has
  mode tabs (stems, MIDI).
- Tests: `resonance-audio/tests/bounce/export_encoders.rs`,
  `export_settings.rs`, `export_normalize.rs`, `delivery_normalize.rs`.

## Open decisions

1. One "Export" dialog with a mixdown tab, or keep Cmd+B as the quick WAV
   bounce? **Recommended:** add a "Mixdown" tab to the existing export dialog
   and keep Cmd+B as the fast default.
2. Persist the last-used settings? **Recommended:** yes, `ExportSettings`
   is already serde.
3. Control API shape: a `format` object mirroring `ExportFormat`.
   **Recommended:** yes, additive, no protocol bump.

## Slices

1. Metadata in WAV/FLAC/MP3 sinks (engine only, tested by reading tags back).
2. Mixdown tab in the export dialog sending `ExportAudio`, with a golden.
3. `render.mixdown` `format` parameter, MCP tool description, skill update
   (`mastering/references/delivery.md`).

## How we'd know it works

Files written from the GUI and from `render_mixdown` decode to the expected
format, rate and length, carry their tags, and report achieved loudness
within 0.5 LU of target.
