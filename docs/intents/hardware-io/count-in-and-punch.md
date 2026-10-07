# Count-in setting and punch recording

> Status: **idea**
> Area: hardware-io
> Source: code reading for [recording](../../features/hardware-io/recording.md): `TransportState::precount_bars` is fixed at 2; the project's `punch_*` fields are aliases of the loop range.

## Why

- Every Record counts in two bars. There is no way to record straight away
  (for example when the performer starts on the downbeat of a running
  loop) or to count in one bar at a slow tempo.
- There is no punch-in/out: the only way to limit a take to a region is to
  loop that region, which also repeats the playback. With "Distinct takes
  per loop pass" off, the single clip shows only the first pass through the
  loop.

## What it would do

- A count-in setting (Off, 1, 2, 4 bars), saved with the user settings or
  the project, used by the GUI Record and the control API.
- A punch region: record only between punch-in and punch-out while the
  transport plays through without looping. Pre-roll from the playhead.

## What exists to build on

- `AudioCommand::Record { precount_bars }` already takes any count; 0
  starts at once (`resonance-audio/src/engine/transport.rs::handle_record_with`).
- Take trimming to a range exists (`RecordingState::finalize_recording`
  trims to the loop range non-destructively).
- `ProjectFile` still accepts `punch_enabled` / `punch_in` / `punch_out`
  as aliases of the loop fields.

## Open decisions

1. Count-in: user setting or project setting? Recommended: user setting
   (it is a habit, not part of the song).
2. Punch: a separate range, or reuse the loop range with a "punch, don't
   cycle" toggle? Recommended: separate range, so loop and punch can
   differ.
3. Should the metronome play during pre-roll when the click is off?
   Recommended: follow the count-in rule (click during count-in only).

## Slices

1. Count-in picker in Settings → Recording, wired to `precount_bars`, plus
   control-API field.
2. Punch range on the ruler + `SetPunchRange` command; the engine trims takes
   to it and does not wrap.
3. Persist punch, undo, golden of the ruler.

## How we'd know it works

Engine tests next to `recording_alignment.rs` for count-in 0/1 and for
punch trimming; app tests for the setting and the ruler; a live take that
punches into a running part.
