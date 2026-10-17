# Drum material per drum track

> Stage: **intent**
> Status: **idea**
> Area: compose
> Source: code reading 2026-10-07 — `materialize_drum_clips_for` writes to every drum track; `generate.drums` takes a `track_id` it only validates.

## Why

A project with two drum tracks (say an acoustic kit and an electronic
layer, or kick/snare split across instruments) gets the **same** derived
drum clip on both: the section's arrangement renders onto every
instrument track whose type is Drum. `generate.drums` requires a
`track_id` but uses it only to check that it is a drum track. An agent
asking for "halftime on the acoustic kit, four-on-floor on the
electronic one" cannot express it.

## What it would do

A section's drum arrangement can target specific drum tracks; different
drum tracks can play different patterns in the same section, and
`generate.drums` writes only to the track it names.

## What exists to build on

- Derived clips are already keyed per track
  (`(definition, placement, track)`), so the map supports it.
- `SectionDefinitionState::arrangement`, `materialize_drum_clips_for`,
  `control_generate_drums` in `update/compose/drum_groups.rs`.
- Feature doc: [drum arrangement](../../features/compose/drum-arrangement.md).

## Open decisions

1. Model: per-(section, drum track) arrangement map, recommended, keeping
   today's single arrangement as the default for tracks without one.
2. GUI: show one drum lane per drum track in Compose (recommended) or a
   track selector on the one lane.

## Slices

1. Model + persistence + `generate.drums` honouring `track_id`; tests in
   `tests/control/control_generate.rs`.
2. Compose view: per-track drum lanes and rail targeting.

## How we'd know it works

Two drum tracks, two `generate.drums` calls with different patterns, and
`song.notes` shows different notes on each.
