# Recording and MIDI Learn: live checks and leftovers

> Stage: **intent**
> Status: **idea** (checklist)
> Area: hardware-io
> Source: [code review 2026-10-02](../../history/reviews/code-review-2026-10-02.md) FU-R1, FU-R1d, FU-R1e, FU-W3a, FU-A1b3, FU-W1c, FU-W1d, FU-W1f

## Why

The recording and MIDI-learn work is proven by offline tests; several
checks need real hardware, and a few code leftovers remain. Recording
defects that are known to be wrong are in [`../bugs.md`](../bugs.md)
(BUG-20…BUG-24), not here. MIDI-learn binding editing and bus/master
targets are in [`midi-learn-binding-editing.md`](midi-learn-binding-editing.md).

## What it would do

Live checks (no code change known):

- [ ] **FU-R1** — on a real interface: count-in into record has no gap at
      the downbeat; an overdub with a latent master plugin sits on the
      grid; cycle-record comps have no flams; behaviour holds when cpal
      opens more channels than requested.
- [ ] **FU-A1b3** — launch the app by hand: native PipeWire output, and
      again with `RESONANCE_FORCE_CPAL_OUTPUT=1`.
- [ ] **FU-W3a** — a test (or live check) of `QueryIoLatency` against real PipeWire.

Code leftovers:

- [ ] **FU-R1d** — the input stream is rebuilt on every record start
      instead of reused (`resonance-audio/src/engine/transport.rs:433-437`).
- [ ] **FU-R1e** — the capture ring is sized from the *requested* channel
      count and engine rate, not the negotiated ones (`transport.rs:443-446`).
- [ ] **FU-W1c** — sub-track mute, solo and pan are not learnable; only
      the fader is (`resonance-app/src/view/mixer/track_strip.rs:457`).
- [ ] **FU-W1d** — bindings to deleted targets stay listed as "Deleted …"
      (`view/midi_learn.rs:126,154`); prune or offer re-target.
- [ ] **FU-W1f** — the control-surface device list is kept twice
      (`engine_events/midi_map.rs:43`, `state/midi_map.rs:32`).

## What exists to build on

[`../../features/hardware-io/recording.md`](../../features/hardware-io/recording.md),
[`../../features/hardware-io/midi-learn.md`](../../features/hardware-io/midi-learn.md),
[`../../features/hardware-io/audio-devices-and-pipewire.md`](../../features/hardware-io/audio-devices-and-pipewire.md).

## Open decisions

None.

## Slices

Live checks in one session on the Scarlett; code items one commit each.

## How we'd know it works

Live: a recorded click track lined up against the grid in the take view.
Code: engine tests for stream reuse and ring sizing.
