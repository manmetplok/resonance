# Automation recording (write / touch / latch)

> Stage: **intent**
> Status: **idea**
> Area: timeline
> Touches: with audio-engine for the capture path
> Source: `docs/history/specs/automation-control-api.md` §8 "Out of scope: recording automation from live control moves"; no write mode exists in code at 8eac71b3.

## Why

Today a lane gets its points only from clicks on the Arrange band, from the
mixer's `+ Add lane` seed point, or from the `automation.*` control API.
Riding a fader, sweeping a plugin knob or turning a MIDI-learned hardware
knob during playback changes the static value but leaves nothing on the
timeline. Real-time moves are the most common way to make automation in
other DAWs, and the user cannot do it here.

## What it would do

- A per-lane (or per-track) automation mode: **Read** (today's `enabled`),
  **Touch** (writes while the control is held, then returns to the lane),
  **Latch** (writes from the first touch until stop) and **Write** (overwrites
  while rolling).
- While the transport rolls, moves of a fader, pan knob, mute button, plugin
  parameter (GUI or plugin editor) or MIDI-learned control write breakpoints
  into the target's lane. A lane is created if none exists.
- The captured points are thinned, so a held knob makes a few points and not
  one per GUI frame.
- One record pass is one undo entry.

## What exists to build on

- Lane model and whole-lane edits: `resonance-common/src/automation.rs`,
  `AutomationMessage::SetLane` in `resonance-app/src/update/automation.rs`
  (one message, one undo entry).
- Gesture-bracketed undo (`StartBreakpointDrag` / `EndBreakpointDrag` →
  `UndoAction::Begin` / `Commit`), a model for a record pass.
- The plugin param-change path from editors (`resonance-app/src/update/plugin.rs`)
  and MIDI Learn (`resonance-app/src/update/midi_map.rs`) already arrive as app
  messages.
- The engine's playhead frame, the engine's own lane sampling
  (`resonance-audio/src/engine/automation.rs`), and `AutomatedValue` for the
  tint.
- See [../../features/timeline/automation-lanes.md](../../features/timeline/automation-lanes.md).

## Open decisions

1. Where to timestamp a move: app-side from the last reported playhead, or
   engine-side. **Recommended:** engine-side, stamping the frame when the
   parameter change reaches the audio thread, so the capture lines up with
   what was heard, not with GUI latency.
2. Mode granularity. **Recommended:** a per-lane mode stored on
   `AutomationLane`, with a per-track "arm automation" toggle in the header
   that sets all its lanes.
3. Thinning. **Recommended:** drop points that are within 1/512 normalized of
   a straight line between neighbours, the same epsilon as
   `AUTOMATED_VALUE_EPSILON`.
4. Control-API exposure. **Recommended:** none at first. Agents write points
   directly.

## Slices

1. Touch mode on track volume only: mode field plus serde default, capture of
   fader drags while rolling, one undo entry per pass, tests in
   `resonance-app/tests/timeline/`.
2. Pan, mute, bus and master.
3. Plugin parameters (GUI knobs and plugin-editor gestures via CLAP
   begin/end gesture).
4. Latch and Write modes. MIDI-learned controls.

## How we'd know it works

- An app test plays the transport, drags a fader through three values and
  asserts the lane points, their frames and a single undo entry.
- A live session: ride a fader, stop, play back, and the fader follows.
