# Quantize toolbar remembers its settings

> Status: **partly built**
> Area: timeline
> Source: ba todo #395 ("last-used quantize / humanize settings"); found unwired while verifying `docs/features/timeline/midi-editor.md` on 2026-10-07.

## Why

`QuantizeState::settings` (`state/quantize.rs`) is saved in the project file
and restored on load, and `tests/timeline/quantize_persistence.rs` covers
the round trip. But the Quantize toolbar reads and writes
`MidiQuantizePanelState`, a separate view-state struct that starts from
defaults every session. Nothing copies one into the other. As a result, a
user who always quantizes to 8th triplets at 80% has to set that up again
each time, and the field saved in the project is never used.

## What it would do

Opening the piano roll shows the last-used grid, strength, swing, mode,
ends/iterative flags, humanize amounts and groove selection. Changing them
updates the project's settings. Undo does not record toolbar tweaks.

## What exists to build on

- `QuantizeSettings` (same fields), `serialize.rs` / `restore.rs` already
  wiring it.
- `MidiEditorMessage::SetQuantize*` handlers.

## Open decisions

1. Which is the source of truth. Recommended: drop the duplicated fields
   from `MidiQuantizePanelState` and have the toolbar bind to
   `Resonance::quantize.settings`, keeping only the transient
   groove-name fields in the panel state.
2. Whether a settings change marks the project dirty. Recommended: yes, but
   with no undo entry.

## Slices

1. Bind the toolbar to the project settings, with a test: set the grid,
   save, load, and the toolbar shows it.

## How we'd know it works

The handler test above passes, and the existing persistence test still
passes.
