# Separate climaxes across simultaneous melodies

> Stage: **intent**
> Status: **idea**
> Area: compose
> Source: `docs/history/specs/todo.md` Tier 4, "Cross-lane climax separation" (§2A: "two simultaneous melodies must not place climaxes together"). Still open at `8eac71b3`: climax placement (`derive/climax.rs`, `section_climax_phrase`) runs inside each generator; nothing in `resonance-app` mentions climaxes.

## Why

Each lane's generator places one climax per phrase and one per section on
its own. A vocal and a lead synth in the same section often peak in the
same bar, which crowds the high register and flattens the shape of the
section.

## What it would do

When several melodic lanes (vocal, lead melody, motif bass) share a
section, their section climaxes land in different phrases or bars; the
vocal gets priority by default.

## What exists to build on

- `section_climax_phrase`, `enforce_single_climax`, vocal
  `apply_section_climax` in `resonance-music-theory`.
- `update/compose/regenerate.rs` already collects vocal spans for
  `fill_vocal_gaps` (`collect_section_vocal_spans`), a model for passing
  cross-lane context into a generator.
- Feature doc: [melody and motif engine](../../features/compose/melody-and-motif-engine.md).

## Open decisions

1. Coordination point: the app passes "reserved climax bars" from
   already-generated lanes into the next generator (recommended, simple),
   or a section-level plan assigns climax phrases to lanes up front.
2. Priority order: vocal > lead > bass (recommended).

## Slices

1. Theory: an optional `avoid_climax_bars` input to the melody and vocal
   generators; tests.
2. App: `regenerate_lane` gathers other lanes' climax bars; integration
   test with a vocal and a lead lane.

## How we'd know it works

A seeded section with a vocal and a lead: the highest note of each falls in
different bars, while each still has exactly one climax.
