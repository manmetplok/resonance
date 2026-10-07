# Make motif augmentation audible (or drop it)

> Status: **idea**
> Area: compose (`resonance-music-theory`)
> Source: `docs/history/specs/todo.md` Tier 4 polish, "Fix Augment/InvertAugment washout" (noted in fd2581b). Still open at `8eac71b3`: `Transform::Augment` doubles `duration_ratio` (`motif_engine/build.rs`) and the realizer divides by the per-chord `total_ratio` (`motif_engine/phrase.rs`), so a uniform doubling cancels out; `plan_motif_transforms` still draws `Augment` (~7% of rolls) and `ComposedPair::InvertAugment`.

## Why

A phrase that rolls `Augment` should sound broader — the motif in longer
notes. Because durations are normalised to fill each chord, it sounds the
same as `Identity` (and `InvertAugment` the same as `Invert`). The draw is
wasted variety, and plan-level tests can count augmentations the listener
never hears.

## What it would do

Either augmentation stretches the motif across more time (e.g. the
augmented cell spans two chord slots, or only its first half is played in
the slot) so it is audible, or `Augment` / `InvertAugment` are removed from
the drawn vocabulary.

## What exists to build on

- `resonance-music-theory/src/derive/motif_engine/{build,phrase,types}.rs`.
- Tests: `composed_transforms.rs`, `phrase_forms.rs`, `rhythm_transforms.rs`.
- Feature doc: [melody and motif engine](../../features/compose/melody-and-motif-engine.md).

## Open decisions

1. Fix vs drop. Recommended: fix by realizing an augmented phrase across
   two chord slots (truncating at the phrase end), which matches the
   classical meaning; drop only if that breaks cadence placement.

## Slices

1. Realizer change + a test asserting augmented notes are longer than the
   identity rendering of the same motif; re-bless affected goldens.

## How we'd know it works

For a fixed seed and motif, the rendered note durations under `Augment`
are about twice those under `Identity`.
