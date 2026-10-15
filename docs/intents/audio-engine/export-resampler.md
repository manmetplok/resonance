# Export resampling: one resampler, no rubato 0.16

> Status: **idea**
> Area: audio-engine
> Source: dependency review 2026-10-15. rubato 0.16 is pinned, and the latest release, 5.0, is a full API rewrite.

## Why

`rubato = "0.16"` is the last direct dependency held back by a major
version (`cargo update --verbose`: 0.16.2, with 5.0.1 available). Upgrading
means porting to an API that was rewritten from scratch, and the port would
change exported audio.

The upgrade is also the wrong question. rubato is used in exactly one
place, `resonance-audio/src/engine/bounce/resample.rs` (`ResampleStage`,
an `FftFixedIn` wrapper), for **export-time** rate conversion. Every other
conversion in the codebase already goes through our own band-limited
resampler in `resonance-common/src/resample.rs` (LIB-01):

- clip import and load-time conversion
- monitor fallback
- IR, drum and wavetable sample loading
- vocal renders

That leaves two resamplers with different behaviour. The export stage
copes with rubato's FFT group delay by skipping output and padding with
zero chunks at the tail (the `flush` loop has a 64-iteration guard). It
also allocates a `Vec` for every chunk. The in-house resampler instead has:

- zero phase: output frame `n` is input time `n·in/out` exactly
- edge extension rather than zero padding
- a stopband of about 90 dB with a passband flat to 0.93 of Nyquist
- tests for quality, streaming and no-allocation (`resonance-common/tests/resample_quality.rs`, `streaming_resample.rs`, `resample_alloc.rs`)

## What it would do

- `ResampleStage` wraps `resonance_common::resample::StreamingResampler`
  (`new(source_rate, target_rate)`, `process(&[f32], &mut Vec<f32>)`,
  `flush`) instead of `rubato::FftFixedIn`. The group-delay skip and the
  zero-chunk drain go away. Output length stays exactly
  `round(in_frames · out/in)`.
- `rubato` is removed from `resonance-audio/Cargo.toml` and `Cargo.lock`.
- When to resample stays the same:
  - when the export's target rate differs from the engine rate
    (`engine/bounce/wav.rs`, the two `ResampleStage::new` sites around :159
    and :204, plus the one around :445);
  - in particular, feeding the Opus encoder, which only takes 48 kHz,
    from a 44.1 kHz engine.

## What exists to build on

- `resonance-common/src/resample.rs`: `StreamingResampler`,
  `ResampleKernel`, `resample_stereo`; documented in
  [audio-file-decoding](../../features/hardware-io/audio-file-decoding.md).
- `resonance-audio/tests/bounce/export_encoders.rs`: the resampled-export
  cases ("Requesting a different sample rate resamples the file", plus a
  non-48 kHz engine feeding Opus).
- [offline-render-and-export](../../features/audio-engine/offline-render-and-export.md)
  lists `ResampleStage` as "a `rubato` FFT stage" and must be updated.
- Related: [export-formats-and-metadata](export-formats-and-metadata.md),
  the intent that exposes the export sample rate in the GUI. Until it
  ships, export resampling is reachable only through the Opus path and
  `ExportSettings` set over the API.

## Open decisions

1. **Replace rubato, or upgrade it to 5.0?** Recommended: replace it. That
   gives one resampler with known, tested quality, removes a dependency,
   and avoids porting to a rewritten API. Upgrade only if an export-specific
   need turns up that the in-house filter can't meet, such as a measured
   quality gap at 44.1 ↔ 48 kHz.
2. **Golden impact.** Exported files that are resampled will change
   slightly: a different filter and no group-delay trim. Recommended:
   accept it. Re-bless only goldens that exercise a rate change, and
   re-check that duration and alignment match the source exactly.

## Slices

One slice: swap the stage, drop the dependency, update the feature doc,
and run the tests below.

## How we'd know it works

- `export_encoders.rs` passes. The existing duration assertion still holds
  exactly: output frames = `round(in · out/in)`.
- New: a 1 kHz tone exported 48 → 44.1 kHz and 44.1 → 48 kHz keeps its
  level within ±0.1 dB, and has no image or alias above −80 dB
  (measured like `resample_quality.rs`).
- New: an impulse at frame `k` lands at `round(k · out/in)` in the export
  (alignment, which the old group-delay skip was approximating).
- `cargo tree -i rubato` finds nothing.
