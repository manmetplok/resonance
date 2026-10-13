# Ranged mixdown and `render.stems` over the control API

> Status: **designed** (engine supports stems and ranged measurement;
> the control layer refuses both; decisions taken 2026-10-13)
> Area: control-api (the methods); the export itself is in audio-engine —
> [offline-render-and-export.md](../../features/audio-engine/offline-render-and-export.md),
> [stem-export.md](../../features/audio-engine/stem-export.md)
> Source: `resonance-control/src/methods/render.rs` (`STEMS` doc comment),
> `resonance-app/src/update/control/render.rs` (partial `range` rejected),
> ba doc #275 P4; checked against master @ `8eac71b3` on 2026-10-07.

## Why

An agent can only bounce the whole song. `render.mixdown` rejects a partial
`range` (`is_whole` check → `unsupported`), and `render.stems` answers
`unsupported` (it is kept out of `control.hello` capabilities), though the
GUI's Export modal does stems. So an agent cannot render "the chorus" for
the user to audition, and a user who asks an agent to "export stems for the
mastering engineer" gets nothing — the agent has to tell them to open the
modal by hand. The MCP server instructions tell agents that a mixdown
always starts at the earliest clip, not bar 1.

## What it would do

- `render.mixdown` with a `range` renders exactly that window (offset 0 =
  the range start).
- `render.stems {dir, range?, overwrite, track_ids?}` writes one WAV per
  track (the modal's sources, or only `track_ids` when given — e.g. just
  the vocal for an intelligibility check) into `dir` as a control job. The job result is
  `StemsResult {paths, duration_s}`. An existing file in `dir` without
  `overwrite: true` answers `needs_confirmation`. The method is listed in
  `capabilities` and published as the MCP tool `render_stems`.

## What exists to build on

- Wire types already defined: `render::STEMS`, `StemsParams`,
  `StemsResult` (`resonance-control/src/methods/render.rs`). `STEMS` is
  deliberately left out of `render::METHODS`.
- App handler stub: `stems()` in
  `resonance-app/src/update/control/render.rs` validates params and
  answers `unsupported` ("stem export lands with the export epic").
- Engine side is built: `AudioCommand::ExportStems { targets, range,
  sample_rate, bit_depth, include_fx_tail }` with per-target
  `StemExportProgress` and completion events
  (`engine/bounce/stem_export.rs`); `MeasureMix` already takes a range.
  `run_export` (`engine/bounce/wav.rs`) computes its own range, and
  `AudioCommand::ExportAudio { path, settings }` has no range field yet.
- `RangeSpec` with `PositionSpec` ends is already the wire shape for
  `range`.
- GUI flow: `resonance-app/src/update/export.rs` (`ExportMessage`,
  Export modal) sends `ExportStems`; `engine_events/export.rs` mirrors
  the events into the modal's phase. It states the modal is the only
  sender, so with the modal closed an event is only logged.
- The mutation gate already treats a running stem export as busy
  (`offline_render_busy_error` → `stem_export_in_progress`).
- Job plumbing to copy: `render.mixdown` (`JobToken::Export { path }`,
  `mixdown_result`, `fail_export_jobs`).

## Decisions

Taken 2026-10-13: all four recommendations below are adopted, and
Jorrit added an optional `track_ids` filter (default every track) so an
agent can bounce a single track.


1. Range on `ExportAudio` or a new field on `ExportSettings`?
   Recommended: an optional `range` on `ExportAudio`, matching
   `ExportStems`.
2. How control-driven and modal-driven exports share the engine events.
   Recommended: a `JobToken::StemExport { dir }` resolved in
   `engine_events/export.rs` next to the modal update, so both paths read
   the same events, and `stem_export_in_progress` also checks for a live
   stem job.
3. Which sources a control call exports. Recommended: the same default
   set as the modal (every track; busses and master optional via a later
   field), to keep one definition of "stems".
4. Bit depth and range defaults. Recommended: the modal's defaults
   (`StemBitDepth` default, whole song), with `range` as in
   `render.mixdown`; stem file naming as the GUI export dialog names them,
   so files match.

## Slices

1. Engine: optional range on `ExportAudio` + bounce test; `render.mixdown`
   accepts a partial `range`.
2. Job token + completion hook + `stem_export_in_progress` covering
   control jobs; handler builds `AudioCommand::ExportStems`; move `STEMS`
   into `METHODS`; MCP tool `render_stems` via `invoke_job`; test in
   `resonance-app/tests/control/control_render.rs`.
3. Progress reporting (`set_progress` from `StemExportProgress`).
4. Mention in the mastering skill's delivery reference, if wanted.

## How we'd know it works

- A control test renders a two-bar range whose length equals two bars plus
  the tail.
- A control test runs `render.stems` on a two-track project and gets two
  non-empty WAVs and a `done` job with both paths, matching the GUI export
  of the same project.
- `combined_router_exposes_every_control_method` passes with the new
  tool; `control_hello` lists `render.stems`.
- Opening the Export modal during a control stem job is refused, and the
  reverse answers `busy`.
