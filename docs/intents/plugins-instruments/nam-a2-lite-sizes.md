# NAM A2-Lite: run slimmable models at reduced size

> Status: **partly built**
> Area: plugins-instruments
> Source: deferred in code, `plugins/resonance-amp/src/nam/parse/container.rs`
> and `nam/wavenet/slimmable.rs` ("runtime A2-Lite size selection is not
> implemented yet"); epic #197 follow-on.
> Register items covered: plugin audit A2.

## Why

Slimmable NAM models can run at a smaller channel count to save CPU at a
small cost in accuracy. Resonance loads them only at full size and refuses
a smaller selection, so a user with many amp instances cannot trade
accuracy for CPU.

## What it would do

A per-instance size choice (for example Full, 3/4, 1/2) for slimmable
models, applied off the audio thread through the existing loader and
crossfaded swap, matching the upstream C++ output at each size.

## What exists to build on

- Slimmable container parsing and full-size slicing
  (`nam/parse/container.rs`, `nam/wavenet/slimmable.rs`).
- Reduced-size reference vectors are already committed and kept intact
  (`tests/nam_a2_reference_parity.rs`,
  `lite_reference_vectors_are_intact_for_future_lite_support`).
- Loader thread and `SwapFader` crossfade (`src/loader.rs`,
  `src/dsp/processor.rs`).
- `docs/features/plugins-instruments/amp-plugin.md`.

## Open decisions

1. Param or state for the size. Recommended: a stepped param, automatable
   off, so agents can set it with `track_set_plugin_param`.
2. Head kernel size above 1 under slimming (not implemented). Recommended:
   refuse with a clear message until a model needs it.

## Slices

1. `derive_params_for_channels` and engine-config rederivation for smaller
   sizes; parity against the slim0 reference vectors.
2. The size param, loader reload on change, editor control.

## How we'd know it works

Parity at each committed reduced size within the A2 tolerance (1e-6);
`nam_rt_safety.rs` still passes; a bench showing the CPU saving.
