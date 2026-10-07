# Recording over the control API

> Status: **idea**
> Area: hardware-io
> Source: `resonance-studio` skill ("The MCP can't set an audio track's input, mono or record-arm"); gap found while documenting [recording](../../features/hardware-io/recording.md).

## Why

An agent running a tracking session can build the backing, but it has to
stop and ask the user to set the input, mono, monitor and arm on every
audio track by hand, and it cannot start a take or switch the click. The
only record-related methods are for external-instrument tracks
(`external.set_record_arm`, `external.set_monitor`, `external.set_return`)
and `transport.set_loop_record_mode`.

## What it would do

- Set a plain audio track's input device and first channel, mono, monitor
  and record-arm; arm an instrument track for MIDI recording.
- Start recording (`transport.record`, with the count-in that the GUI uses)
  and toggle the metronome.
- Report the armed tracks, the current input stream source and the last
  take in a read view, so the agent can check a take landed.

Done when an agent can set up and record a take without the user touching
the track header, and the GUI shows the same state.

## What exists to build on

- The GUI messages already exist and are undoable:
  `TrackMessage::{SetTrackInputDevice, SetTrackInputPort, ToggleTrackMono,
  ToggleMonitor, ToggleRecordArm}`, `TransportMessage::{Record,
  ToggleMetronome}` (`resonance-app/src/update/track.rs`,
  `update/transport.rs`).
- `resonance-app/src/update/control/external.rs` shows the pattern
  (`set_monitor`, `set_record_arm` route through `run_via_update`).
- `external.devices` already lists audio inputs by name.

## Open decisions

1. One `track.set_input` method with device/port/mono, or three setters?
   Recommended: one method with optional fields, mirroring `external.set_return`.
2. Should `transport.record` refuse when nothing is armed (as the GUI
   button does)? Recommended: refuse with a clear error.
3. Recording is audible and writes files: require a `confirm` flag?
   Recommended: no; arming is the deliberate step.

## Slices

1. `track.set_input` + `track.set_monitor` + `track.set_record_arm` (wire,
   handler, MCP tool, control test).
2. `transport.record` + `transport.set_metronome` with the echo in
   `TransportResult`.
3. Read view: armed tracks and input source in `song.tracks`.

## How we'd know it works

Control tests in `resonance-app/tests/control/` that arm, record (with the
fake input harness from `resonance-audio/tests/engine/recording_alignment.rs`
on the engine side) and read back; a live session where the studio skill
records a take without manual steps.
