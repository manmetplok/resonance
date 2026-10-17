# Audio device settings

> Stage: **intent**
> Status: **idea**
> Area: hardware-io
> Source: code reading for [audio devices and PipeWire](../../features/hardware-io/audio-devices-and-pipewire.md); memory note "engine reads quantum at startup".

## Why

- Output always goes to the default sink; choosing another output means
  changing the system default.
- The PipeWire quantum is read once at startup. Switching between the
  tracking quantum (64) and the mixing quantum (128 or larger) needs a
  restart of the app, and nothing in the app shows the quantum it sized its
  buffers for.
- There is no audio hotplug: the input list refreshes only at startup, on
  the bounce dialog and on the external-instrument re-scan.
- On macOS (cpal) the rate is whatever CoreAudio's default config gives.

## What it would do

A Settings → Audio page that shows the engine rate, the quantum in use and
the graph latency (already there), lets the user pick an output device and
a buffer size (writing `clock.force-quantum` like the rate force, restored
on exit), and refreshes the input list when it opens. The engine follows a
quantum change without a restart.

## What exists to build on

- `GraphRateForce` (`resonance-audio/src/platform.rs`) shows the
  write/verify/restore pattern for PipeWire settings metadata.
- Buffers are sized in `audio_engine.rs::probe_output`
  (`max(max_quantum, 2 × quantum, 256)`).
- The latency rows in `view/settings.rs`.

## Open decisions

1. Should the app write `clock.force-quantum` at all, or only display it?
   Recommended: offer it, with restore on exit, as for the rate; never
   change it without the user asking.
2. Output device: target a node by name on the native stream, or leave it
   to the session manager? Recommended: target by name, default "system
   default".

## Slices

1. Read-only rate/quantum display + input-list refresh on open.
2. Live quantum change: re-probe and resize buffers on a graph change event.
3. Buffer-size picker (force-quantum).
4. Output device picker.

## How we'd know it works

Unit tests for the force/restore rules (like `graph_rate_assert.rs`); a live
check switching 128 → 64 → 128 with `pw-top` showing no xruns and the app
following without a restart.
