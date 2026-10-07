# Freeze in the control API

> Status: **idea**
> Area: timeline (see `docs/features/timeline/track-freeze-ui.md`)
> Source: found while documenting freeze, 2026-10-07

## Why

Agents can see that a track is frozen (`song.tracks` → `frozen`) and get
`busy` when they try to edit it, but cannot unfreeze it to make the edit, nor
freeze heavy instrument tracks to save CPU during a long session.

## What it would do

`track.freeze`, `track.unfreeze`, `track.refreeze` and `track.freeze_all`, as
jobs (the render is long-running, like `render.mixdown`), reporting the final
status (`frozen`, `stale`, `failed` with message). `song.tracks` reports
`stale` separately from `frozen`.

## What exists to build on

- `resonance-app/src/update/freeze.rs` (`FreezeMessage`, `freezable`,
  refusal reasons) and the job machinery in `resonance-app/src/control_jobs.rs`.
- `FreezeStatus` in `resonance-app/src/state/freeze.rs`.

## Open decisions

1. Job-based (recommended) vs. fire-and-poll `song.tracks`.
2. While a freeze job runs, other mutating calls answer `busy` (the GUI gate
   already blocks everything else).

## Slices

1. `track.unfreeze` (synchronous) + `stale` in `song.tracks`.
2. `track.freeze` / `refreeze` as jobs; `freeze_all`.

## How we'd know it works

Control tests freeze a track through the hermetic engine stub, see `frozen`,
get `busy` on a note edit, unfreeze, and make the edit; undo reverses each.
