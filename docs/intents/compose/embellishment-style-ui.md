# EmbellishmentStyle control in the melody inspector

> Stage: **intent**
> Status: **partly built**
> Area: compose
> Source: `docs/history/specs/todo.md` Tier 4, "EmbellishmentStyle UI wiring" (knob added in 20d3581). Still open at `8eac71b3`: no file under `resonance-app/src` mentions `embellishment` / `EmbellishmentStyle`.

## Why

`MelodyParams::embellishment` (Auto, Folk, PopBallad, Jazz) changes which
embellishing tones decorate a motif melody — passing/neighbour for folk,
suspensions/appoggiaturas for ballads, anticipations and escape tones for jazz. GUI users
cannot set it; it stays on Auto. Agents can, by passing
`options: {"embellishment": "Jazz"}` to `generate.part` or
`section.set_lane_generator`, but nothing tells them it exists.

## What it would do

A pick list in the melody lane inspector (and visible in the lane's
params readout) that sets the style and regenerates on Regenerate, like
the other melody params.

## What exists to build on

- `resonance-music-theory/src/derive/melody.rs` (`EmbellishmentStyle`,
  `MelodyParams::embellishment`), `derive/motif_engine/embellish.rs`.
- `update/compose/lane_inspector/melody_params.rs` setters,
  `view/compose/lane_inspector/instrument/melody.rs`.
- The schema-generator wiring (`ChordInspectorMsg::SetSchemaKind` + goldens
  `compose_chord_rail_generator_schema`) is the pattern to copy.
- Feature doc: [melody and motif engine](../../features/compose/melody-and-motif-engine.md).

## Open decisions

1. Show it only for `MelodyStyle::Motif` (where it has effect —
   recommended) or always.

## Slices

1. `LaneInspectorMsg::SetMelodyEmbellishment` + setter + cached pick list
   options; update-flow test; golden of the melody rail.

## How we'd know it works

Update test: the message sets the field; golden shows the control; a seeded
regenerate with Folk vs Jazz yields different notes.
