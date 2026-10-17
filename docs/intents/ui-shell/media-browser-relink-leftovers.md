# Media browser and relink leftovers

> Stage: **intent**
> Status: **partly built**
> Area: ui-shell
> Touches: browser with timeline (pool, relink) and control-api
> Source: prototype [`../prototypes/audio-sample-import-media-browser/`](../prototypes/audio-sample-import-media-browser/DESIGN.md);
> [code review 2026-10-02](../../history/reviews/code-review-2026-10-02.md) FU-W4b, FU-W4c

## Why

The parts of the media-browser prototype not covered by
[`media-browser-drag-and-pool-audition.md`](media-browser-drag-and-pool-audition.md)
(drag and pool audition) or
[`../timeline/pool-backed-clip-audio.md`](../timeline/pool-backed-clip-audio.md):

- Pool thumbnails are flat after a project reload (peaks are not rebuilt).
- The Files tab cannot sort, and cannot multi-select to import several
  files at once.
- A clip whose audio is missing looks like any other clip on the
  timeline (FU-W4b); only the relink dialog knows.
- There is no `clip.relink` control method and no `missing` flag in
  `song.tracks`, so an agent cannot find or fix missing audio (FU-W4c).

## What it would do

- Pool peaks rebuilt (or loaded from the peaks cache) on project load.
- Files tab: sort by name / date / length; shift/ctrl multi-select;
  import all selected.
- Missing clips drawn hatched with a "missing" badge; click opens relink.
- `clip.relink` (or `pool.relink`) + a `missing` flag on clip and pool reads.

## What exists to build on

- [`../../features/timeline/audio-pool-and-import.md`](../../features/timeline/audio-pool-and-import.md),
  [`../../features/timeline/relink.md`](../../features/timeline/relink.md),
  [`../../features/ui-shell/media-browser-panel.md`](../../features/ui-shell/media-browser-panel.md).
- `resonance-dsp/src/peaks.rs`, `resonance-app/src/update/relink.rs`,
  `resonance-app/src/update/pool.rs`.
- Undo of a relink is buggy today: BUG-16 in [`../bugs.md`](../bugs.md).

## Open decisions

1. Relink method scope: per clip or per pool asset. **Recommended:** per
   pool asset (one missing file usually backs several clips).

## Slices

1. Pool peaks on load (+ UI snapshot golden).
2. Missing-clip drawing.
3. Files sort + multi-select import.
4. `relink` method + `missing` flag (wire + handler + MCP + test).

## How we'd know it works

iced snapshot goldens for the pool and a missing clip; a control test
relinking a missing asset and reading `missing: false` back.
