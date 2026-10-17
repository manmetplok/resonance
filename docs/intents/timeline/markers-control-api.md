# Markers in the control API

> Stage: **intent**
> Status: **idea**
> Area: timeline
> Touches: see `docs/features/timeline/markers.md`
> Source: found while documenting markers, 2026-10-07

## Why

Agents cannot see or write arrangement markers. They can move them indirectly
(`arrangement.insert_bars` reports `markers_moved`) but cannot read where
the user marked "Chorus 2", drop navigation markers for the user, or loop a
region. `song.summary` does not list them.

## What it would do

`marker.list`, `marker.add` (point or region, 1-based bar/beat position),
`marker.edit` (name, colour, start, end), `marker.remove`, and
`marker.seed_from_sections`; markers listed in `song.summary`. Positions use
the same `SongPosition` shape as the other methods.

## What exists to build on

- `resonance-app/src/update/marker.rs` (`MarkerMessage::{AddAtPlayhead,
  Rename, Recolor, Delete, MoveStart, SetRegionEnd, SeedFromSections}`) — but
  `AddAtPlayhead` and `MoveStart` snap to the zoom-dependent grid, so a wire
  add needs an explicit-position variant.
- Control handler pattern: `resonance-app/src/update/control/global.rs`
  (validate, `run_via_update`, ack with revision).

## Open decisions

1. Address markers by id (recommended; ids are stable, app-allocated) rather
   than by position.
2. Add a `MarkerMessage::Add { start, end, name, color }` variant that does not
   snap (recommended) instead of reusing `AddAtPlayhead`.

## Slices

1. `marker.list` + summary field (read-only).
2. `marker.add` / `edit` / `remove` with the new message, undo test, MCP tools,
   lockstep test entry.
3. `marker.seed_from_sections`.

## How we'd know it works

Control tests in `resonance-app/tests/control/` add, edit and remove a marker,
read it back, and undo each call in one step; GUI ruler shows the result.
