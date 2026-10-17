# Sample-accurate hardware MIDI output

> Stage: **intent**
> Status: **idea**
> Area: hardware-io
> Source: `resonance-audio/src/engine/midi/outbound.rs` ("Once per engine-thread iteration (~16 ms) we look at the playhead delta"), `engine/midi/clock.rs::poll_midi_clock_send`, `poll_device_param_automation`.

## Why

Notes sent to hardware synths, MIDI clock pulses and device-parameter CCs
all leave from the engine control thread, which wakes about every 16 ms and
also runs every blocking command handler. A note can go out up to one
iteration late, with jitter, and a busy engine thread delays it further.
Clock pulses at 24 PPQN (about 20 ms apart at 120 BPM) are emitted in
bursts. The external-instrument latency offset compensates a fixed delay,
not jitter, and is measured with a ping sent outside this path.

## What it would do

Hardware MIDI events leave at the sample position they belong to, within
about a millisecond, independent of engine-thread load: schedule from the
audio callback's block timing into a queue that a dedicated MIDI thread
sends at the right wall-clock time (or with ALSA sequencer timestamps),
and include the output latency so hardware lines up with what is heard.

## What exists to build on

- `outbound_step_start` and `emit_outbound_notes` already compute which
  notes fall in a span; they would run on block spans instead of poll
  spans.
- `OutboundNoteSink` and `DeviceParamMidiSink` are traits, so the sending
  side can be swapped.
- The graph's playback latency is known (`SharedState::playback_latency_samples`).

## Open decisions

1. Timestamped ALSA sequencer queue vs. a high-priority sender thread with
   sleep-until? Recommended: start with the sender thread (portable to
   CoreMIDI); use sequencer timestamps on Linux if jitter stays high.
2. Compensate output latency by sending MIDI early? Recommended: yes,
   behind the existing PDC model.

## Slices

1. Block-timed event queue filled from the mixer; sender thread; notes only.
2. Clock pulses and device-param CCs on the same queue.
3. Latency alignment with the audio output.

## How we'd know it works

Unit tests for the scheduler; a loopback measurement (MIDI out → synth →
audio in) showing note-onset spread before and after; MIDI clock jitter
measured with a hardware sequencer.
