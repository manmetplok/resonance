# Takes in the control API

> Stage: **intent**
> Status: **idea**
> Area: timeline
> Touches: see `docs/features/timeline/takes-and-comping.md`
> Source: `SetLoopRecordModeParams` doc in `resonance-control/src/methods/transport.rs` promises "take methods" that do not exist

## Why

An agent can switch cycle-record mode (`transport.set_loop_record_mode`) but
then cannot see the takes it produced, solo one, comp across them or delete a
bad one. The parameter's own doc tells clients to "comp or solo them
afterwards with the take methods".

## What it would do

- `takes.list` — groups per track with slot, takes (id, pass index, extent,
  audio/MIDI, missing flag), comp segments, active take; also in `song.tracks`.
- `takes.set_active`, `takes.promote` (take, range), `takes.split` (at a
  position, not only the playhead), `takes.delete`.

## What exists to build on

- `resonance-app/src/update/takes.rs`: `TakeMessage` and the pure `plan`
  function, with refusals checked before dispatch
  (`Resonance::take_edit_is_refused`).
- `resonance-app/src/state/takes.rs`: the mirror to read from.

## Open decisions

1. Split at an explicit position (recommended) — add a variant instead of
   moving the playhead.
2. Refused edits answer `invalid_params` with the reason `plan` refused
   (recommended), not a silent ok.

## Slices

1. `takes.list` + `song.tracks` field.
2. `set_active` / `promote` / `split` / `delete`, each with undo and refusal
   tests, MCP tools, lockstep entry.

## How we'd know it works

Control tests drive a mirrored group (as `take_comp_edits.rs` does), read it
back through `takes.list`, and undo each call in one step.
