# Stem export options, overwrite prompt and retry

> Status: **partly built**
> Area: audio-engine (export) with ui-shell (export dialog)
> Source: prototype [`../prototypes/multitrack-stem-and-midi-export/`](../prototypes/multitrack-stem-and-midi-export/index.html); review 2026-10-02 FU-A1a7

## Why

The Audio stems tab of the export dialog works (track / bus / master
checklist, whole project or loop range, destination, progress, cancel,
partial-failure report), but the prototype's option row never got a
surface. Stems always export at the engine defaults, the "Ask each"
overwrite mode silently behaves like "Add suffix" (BUG-64 in
[`../bugs.md`](../bugs.md)), and after a partial failure the only way to
get the missing stems is to export everything again.

## What it would do

- An options row on the stems tab: bit depth, sample rate and "Include FX
  tail", applied by the stem render.
- Filename conflicts: Add suffix `_1` / Overwrite / Ask each, where Ask
  really prompts per file (overwrite, skip, rename).
- "Retry remaining (N)" after a partial failure, re-rendering only the
  stems that failed.
- Settings remembered per project like the mixdown's.

Done when each option changes the written files and a test pins it.

## What exists to build on

- `resonance-app/src/state/export.rs` already holds the stem format and
  `ExportOverwrite` fields; no message or view sets them.
- `resonance-app/src/update/export.rs:205,234` — where `Ask` falls through
  to suffix.
- Engine stem render: `resonance-audio/src/engine/bounce/stem.rs`.
- Feature doc: [`../../features/audio-engine/stem-export.md`](../../features/audio-engine/stem-export.md).
- MIDI tab: [`midi-project-export.md`](midi-project-export.md); control
  API `render.stems` (built, tracks only, the dialog's defaults):
  [`../../features/audio-engine/stem-export.md`](../../features/audio-engine/stem-export.md).

## Open decisions

1. Ask-each prompt: one modal per conflict, or one list of all conflicts
   up front. **Recommended:** one list up front — the render then runs
   unattended.
2. Share the format controls with the mixdown dialog
   ([`export-formats-and-metadata.md`](export-formats-and-metadata.md)).
   **Recommended:** yes, one widget.

## Slices

1. Options row → `ExportStems` parameters → engine; tests on the written WAV headers.
2. Conflict list + real Ask; test with a pre-existing file.
3. Retry remaining from the failure report.

## How we'd know it works

App-group tests on the export flow with the capturing engine, and an
engine bounce test asserting format, rate and tail length of the files.
