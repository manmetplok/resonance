# Drum groove feel: fills, step velocity, control params

> Status: **partly built** — swing and humanize shipped (BUG-51, see
> [drum arrangement](../../features/compose/drum-arrangement.md)).
> Area: compose
> Source: code reading 2026-10-07 — `DrumGroup::fills` is stored but read nowhere.

## Why

The drum rail lists "Fills (last bar)" as not built: `DrumGroup::fills`
is stored but nothing generates a fill from it. Per-step cell values (the
0..=100 velocity scale the built-in grooves author) are treated as on/off,
so ghost notes and accents in a groove are lost. Agents cannot reach
swing or humanize at all — `generate.drums` takes neither.

## What it would do

- **Fills:** chance of a generated fill on the last bar of each phrase or
  entry when no explicit fill pattern is set; the rail gets its slider
  back.
- **Step velocity:** cell value scales velocity.
- **Control:** optional `swing` / `humanize` on `generate.drums` (they
  apply at render time already).

## What exists to build on

- `update/compose/drum_groups.rs`: `generate_group_pattern`,
  `build_drum_notes`, `emit_span_notes`; group seeds; `library.rs`
  authored velocities.
- Arrangement fills (`PatternEntry::fill`) already replace a last bar.
- Feature doc: [drum arrangement](../../features/compose/drum-arrangement.md).

## Open decisions

1. Should `generate.drums` take `swing` / `humanize`? Recommended: yes, as
   optional params written onto the section's (cloned) groups.

## Slices

1. Step velocity in `emit_span_notes`; unit tests on `build_drum_notes`.
2. Generated fills; restore the rail slider.
3. Control params on `generate.drums`.

## How we'd know it works

A built-in groove's ghost notes come out quieter; a group with fills > 0
plays a different last bar, identical for the same seed.
