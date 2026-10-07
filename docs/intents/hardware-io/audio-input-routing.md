# Audio input routing beyond one device

> Status: **idea**
> Area: hardware-io
> Source: code reading of `resonance-audio/src/engine/tracks.rs::sync_input_stream` and `engine/transport.rs::open_recording_session` while documenting [audio input and monitoring](../../features/hardware-io/audio-input-and-monitoring.md).

## Why

- The engine opens one capture stream. Its device is the first monitored
  (or first armed) track's device; every other track reads its channels
  from that stream whatever device it names. Two interfaces, or a USB mic
  next to an interface, cannot be recorded together, and nothing tells the
  user that the second track's device was ignored.
- While a stream is open, changing a monitored track's input device does not
  switch the stream unless more channels are needed.
- Takes are placed with the graph-reported capture + playback latency only.
  Converter and USB latency the graph does not report shift every take by a
  fixed amount, and there is no record-offset setting or loopback
  calibration for audio tracks (external instruments have the ping).

## What it would do

- Open one capture stream per distinct device in use (or refuse with a
  visible message when two are armed), and reopen when a monitored track's
  device changes.
- A per-machine record offset (samples), optionally measured with a
  loopback ping like the external-instrument detector.

## What exists to build on

- `InputHandle`, `platform::build_input_stream`, the monitor ring and
  `MonitorDrain` (`resonance-audio/src/mixer/monitor.rs`).
- The latency ping (`resonance-audio/src/engine/external_instrument_ping.rs`)
  measures a round trip through the same capture path.
- `apply_recording_start_latch` is the one place the take offset is applied.

## Open decisions

1. Multi-device: true multi-stream (clock drift between devices) or a clear
   refusal? Recommended: refuse with a banner first; multi-stream only if a
   real need appears (PipeWire can aggregate devices itself).
2. Record offset: user setting or per input device? Recommended: per device,
   stored in user settings.

## Slices

1. Reopen on device change while monitoring; warn when armed/monitored
   tracks name different devices.
2. Record-offset setting applied in `apply_recording_start_latch`.
3. Loopback calibration button reusing the ping detector.

## How we'd know it works

Engine tests for `sync_input_stream` device switches with a fake
enumerator; an alignment test with a non-zero offset; a live loopback
recording whose click lands on the grid.
