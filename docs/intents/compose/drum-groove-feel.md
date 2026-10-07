# Drum groove feel: swing, humanize, fills, step velocity

> Status: **partly built**
> Area: compose
> Source: code reading 2026-10-07 — `DrumGroup::{swing, humanize, fills}` set by sliders in `view/compose/lane_inspector/drums/rhythm.rs` but read nowhere.

## Why

The drum rail shows Swing, Humanize and "Fills (last bar)" sliders per
group. Moving them changes nothing: `generate_group_pattern` ignores them
and `emit_span_notes` only applies `accent`. Per-step cell values (the
0..=100 velocity scale the built-in grooves author) are treated as on/off,
so ghost notes and accents in a groove are lost. Users and agents get
mechanical drums regardless of settings.

## What it would do

- **Swing:** delay off-beat steps (per the group's grid) by a fraction of
  a step.
- **Humanize:** seeded, reproducible small timing and velocity jitter.
- **Fills:** chance of a generated fill on the last bar of each phrase or
  entry when no explicit fill pattern is set.
- **Step velocity:** cell value scales velocity.

## What exists to build on

- `update/compose/drum_groups.rs`: `generate_group_pattern`,
  `build_drum_notes`, `emit_span_notes`; group seeds; `library.rs`
  authored velocities.
- Arrangement fills (`PatternEntry::fill`) already replace a last bar.
- Feature doc: [drum arrangement](../../features/compose/drum-arrangement.md).

## Open decisions

1. Apply feel at render (materialize) time rather than baking into
   pattern cells — recommended, so editing a cell never fights the feel.
2. Should `generate.drums` take `swing` / `humanize`? Recommended: yes, as
   optional params.

## Slices

1. Step velocity + swing in `emit_span_notes`; unit tests on
   `build_drum_notes`.
2. Humanize (seeded); determinism test.
3. Generated fills; control params on `generate.drums`.

## How we'd know it works

`build_drum_notes` tests show shifted off-beats for swing > 0 and identical
output for the same seed; a built-in groove's ghost notes come out quieter.
