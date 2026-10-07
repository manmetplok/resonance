# Hardware MIDI input, output and clock

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none in `docs/history/specs/`. Leads: doc #260 finding #16 (audio-thread live-MIDI pickup), code review HOST-13, FU-D6a, commit `ea29dfb6` (long-lived enumeration clients).

Instrument and vocal tracks can take notes from a hardware MIDI input and
send their timeline notes to a hardware MIDI output. Live input plays the
track's instrument within one audio quantum, is recorded when the track is
armed, and is echoed to the track's MIDI output (Thru). Resonance can also
send or follow MIDI clock. Ports are listed through ALSA sequencer /
CoreMIDI via `midir`.

## Behaviour

### Ports

- `enumerate_midi_inputs` / `enumerate_midi_outputs` list ports through one
  sequencer client per direction, created on first use and kept for the
  life of the process (`ENUMERATE_IN` / `ENUMERATE_OUT`). Creating a client
  per call made every MIDI bridge on the system, PipeWire's included, see a
  client come and go twice a second (commit `ea29dfb6`).
- The app asks for both lists at startup and then every 2 s
  (`update/tick.rs::refresh_midi_devices_if_stale`), so a newly plugged
  controller shows up in the pickers. The engine sends a list event only
  when it changed.
- A picker keeps showing a configured port that is unplugged
  (`midi_choices_with_override`).

### Per-track input

- Mix inspector → ROUTING → MIDI INPUT and MIDI IN CHANNEL (Omni or 1–16)
  on instrument and vocal tracks. Saved in the project, undoable.
- One midir connection per track. A device that is not present is kept as
  *pending* and opened on the next poll that finds it
  (`MidiInputRegistry::reconcile`). Changing only the channel swaps the
  filter in place without reopening.
- The midir thread parses each message, applies the channel filter and
  pushes a `LiveMidiEvent` (with its arrival `Instant`) into a bounded
  channel (1024). The **audio callback** drains it and queues the note to
  the track's instrument with an intra-block offset estimated from the
  arrival time (`live_arrival_sample_offset`), then forwards the event to
  the engine thread for recording and Thru.
- Controllers, pitch bend and aftertouch (`InboundMidi`) go to the
  instrument as raw MIDI when its note port accepts MIDI. They are not
  recorded and not sent Thru.

### Per-track output and Thru

- ROUTING → MIDI OUTPUT and MIDI OUT CHANNEL (1–16, default 1) on
  instrument and vocal tracks (external-instrument tracks set it in their
  own section, see [external-instruments.md](external-instruments.md)).
- Output connections are shared per port and reference-counted. Reassigning
  a track sends All Notes Off on the old port. A port that is not present
  when it is picked fails with an error banner and is not retried; pick it
  again after plugging it in.
- **Timeline → hardware:** each engine-thread iteration (~16 ms) looks at
  the playhead span since the last poll and sends note-ons for notes that
  started in it and note-offs for held notes that ended in it
  (`poll_timeline_to_midi_output`). A jump of more than one second, or a
  backward jump, is treated as a seek or loop wrap: held notes are released,
  and after a loop wrap notes from the loop start are sent. Muted tracks send
  nothing. Hardware notes are therefore quantized to the engine-thread poll
  and are not delay-compensated.
- **Thru:** live input notes on a MIDI-accepting track go out on that track's
  MIDI output and channel.
- Stop, Pause and seeks send All Notes Off to every output.

### MIDI clock

Settings → General → **MIDI Clock**: "Send MIDI Clock" with an output port,
"Receive MIDI Clock" with an input port. Both are saved in the project.

- **Send:** 24 PPQN pulses follow the tempo map, emitted from the engine
  thread's poll. Play from 0 sends Start, from elsewhere Continue plus Song
  Position; Pause/Stop send Stop; a seek and Stop send Song Position.
- **Receive:** Start sets the playhead to 0 and plays; Continue plays; Stop
  pauses; Song Position seeks. While the master runs, clock pulses feed a
  tempo estimator; changes of more than 0.1 BPM set the engine's flat BPM
  and the app's BPM field (`MidiClockTempoDetected`). The playhead is not
  phase-locked to the pulses, and the tempo map's events are not rewritten.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Enumeration, connections, encoding | `resonance-audio/src/midi_hardware.rs` | `enumerate_midi_inputs`, `MidiInputRegistry`, `MidiOutputRegistry`, `LiveMidiEvent`, `encode_control_change`, `encode_nrpn` |
| Engine handlers | `resonance-audio/src/engine/midi/hardware.rs`, `engine/midi/live.rs`, `engine/midi/outbound.rs`, `engine/midi/state.rs` | `handle_list_midi_inputs`, `handle_set_track_midi_input`, `handle_live_midi_event`, `poll_timeline_to_midi_output`, `outbound_step_start` |
| Audio-thread pickup | `resonance-audio/src/mixer/live_midi.rs` | `pickup_live_midi` |
| Clock | `resonance-audio/src/midi_clock.rs`, `engine/midi/clock.rs` | `poll_midi_clock_send`, `handle_midi_clock_event` |
| App | `resonance-app/src/state/midi_devices.rs`, `update/track.rs`, `update/transport.rs`, `update/tick.rs` | `MidiDevices`, `SetTrackMidiInputDevice`, `ToggleMidiClockSend` |
| View | `resonance-app/src/view/mixer/inspector/io.rs`, `view/settings.rs` | `midi_input_block`, `midi_output_block` |

The audio thread never calls midir; only the engine thread opens, closes or
sends.

## Surfaces

- **GUI:** Mix inspector → ROUTING (MIDI in/out + channels); Settings →
  General → MIDI Clock. The control-surface port for MIDI Learn is on
  Settings → MIDI ([midi-learn.md](midi-learn.md)).
- **Control API / MCP:** none for track MIDI routing or clock.
  `external.devices` / `external.set_midi_out` cover external-instrument
  tracks only.

## How it's tested

| What | Where | Run |
|---|---|---|
| Message parsing / encoding | `resonance-audio/tests/midi_hw/midi_hardware_parse.rs`, `midi_hardware_emit.rs`, `midi_program_change.rs`, `control_surface_parse.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Long-lived enumeration clients | `resonance-audio/tests/midi_hw/enumerate_clients.rs` | same |
| Live arrival offset, retry order, record stop | `resonance-audio/tests/midi_hw/live_arrival_offset.rs`, `live_note_retry_order.rs`, `live_record_stop.rs` | same |
| Outbound stepping and note pairing | `resonance-audio/tests/midi_hw/outbound_step_start.rs`, `outbound_note_pairing.rs` | same |
| Clock parsing | `resonance-audio/tests/midi_hw/midi_clock_parse.rs` | same |

Not covered: a real port end to end, clock send timing, the 2 s hotplug
poll, and the input/output pickers in the inspector (no golden).

## Decisions and deviations

- Live notes reach the instrument from the audio callback (doc #260
  finding #16); they used to wait for the engine thread, about 16 ms.
- Outbound hardware notes stay on the engine thread so midir's locks never
  touch the audio callback; the cost is ~16 ms timing granularity.

## Known gaps

- Hardware MIDI out and clock are quantized to the engine poll:
  [sample-accurate-midi-out](../../intents/hardware-io/sample-accurate-midi-out.md).
- Controllers are not recorded or sent Thru:
  [midi-controller-data](../../intents/hardware-io/midi-controller-data.md).
