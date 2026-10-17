# MIDI controller data: record, Thru, import

> Stage: **intent**
> Status: **idea**
> Area: hardware-io
> Touches: clip storage: timeline
> Source: `resonance-audio/src/engine/midi/live.rs` ("Controllers reach the instrument on the audio thread (HOST-13); recording them and sending them Thru is not built yet"); `midi_io.rs` imports notes only.
> Register items covered: FU-H1a (review 2026-10-02, what HOST-13 left).

## Why

Sustain pedal, mod wheel, pitch bend and aftertouch from a keyboard play the
instrument live, but they vanish from the recording: playback of a recorded
part has no sustain and no bends. They are not sent Thru to a hardware
synth either, and an imported `.mid` file loses them.

## What it would do

- Record CC, pitch bend, channel and poly aftertouch into the MIDI clip with
  the notes, play them back to the instrument and to hardware outputs.
- Send them Thru with the notes.
- Import them from SMF files.

## What exists to build on

- `LiveMidiEvent::InboundMidi { data, arrival }` already carries the raw
  message to the audio thread (`mixer/live_midi.rs`) and on to the engine
  thread, where it is currently dropped.
- Instruments accept raw MIDI as `CLAP_EVENT_MIDI` when their note port does.
- `MidiOutputRegistry::send_control_change` exists.
- `MidiClip` stores notes only; this needs a controller-event list in the
  model, the clip `.mid` files and the piano roll (timeline area).

## Open decisions

1. Store controllers as per-clip MIDI events or as automation lanes?
   Recommended: per-clip events (they belong to the performance), with a
   later "convert to automation" action.
2. Show them in the piano roll from the start? Recommended: record and play
   first, editing later.

## Slices

1. Model + clip file round trip for controller events.
2. Record from live input; play back in the mixer.
3. Thru and hardware output.
4. SMF import of controllers.
5. Piano-roll lane to view and edit them (timeline).

## How we'd know it works

Engine tests next to `live_record_stop.rs` for recorded CC/bend; SMF tests
in `smf_import.rs`; a live take with sustain pedal that plays back the same.
