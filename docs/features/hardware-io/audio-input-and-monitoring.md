# Audio input and monitoring

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none in `docs/history/specs/`. Leads: doc #260 (findings #2, #16, #17, #21), code review RT-14 / RT-17 / MIX-09.

An audio track picks an input device and a starting input channel, can be
switched to mono, and can be monitored: the live input then plays through
the track's own plugin chain, fader, sends and bus, while stopped as well as
while playing. Record-arm uses the same input (see [recording.md](recording.md)).

## Behaviour

### Per-track input

- Each audio track stores `input_device_name` and `input_port_index`
  (0-based first channel). Mono tracks capture that channel; stereo tracks
  capture it as L and the next one as R. Picking a new device resets the
  channel to 0.
- The channel picker lists the channels the device reports
  (`InputDeviceInfo::channels`): every channel for a mono track, every
  stereo pair start for a stereo one.
- Arming a track that has no input picks the OS default source.
- Instrument and vocal tracks show a MIDI input instead
  ([midi-hardware-io.md](midi-hardware-io.md)). External-instrument tracks
  set their input as the "audio return"
  ([external-instruments.md](external-instruments.md)).
- Input device, channel, mono, monitor and arm are project state: saved in
  `ProjectTrack` and each change is one undo entry.

### One input stream at a time

The engine holds a single capture stream (`state.rec.input_stream`),
opened by `sync_input_stream` whenever a track is monitored or armed:

- Its source is the device of the **first** monitor-enabled track found;
  for a record session, the first armed track that names a device. Every
  other monitored or armed track reads its channels from that same stream,
  whatever device it names.
- It is opened wide enough for the highest channel any monitored track needs
  (`port + 1` mono, `port + 2` stereo; at least 2, at most 32).
- It is rebuilt only when there is no stream or it has too few channels.
  Changing a monitored track's device while a stream is open therefore does
  not switch the source until monitoring is turned off and on again (code
  reading; not covered by a test).
- During a recording the stream is left alone; only record start may
  rebuild it.
- With no monitored and no armed track the stream is closed.

### Monitoring

- **Stopped or counting in:** `mixer/monitor.rs::mix_monitor_passthrough`
  de-interleaves the stream, routes each monitored track's channel(s) to its
  L/R pair, runs its chain and fader, and sends the result to its bus or the
  master and its aux sends. The bus pass and master chain then run, so the
  performer hears the same reverb send and master chain as while rolling.
- **Playing:** monitored input is mixed inside the timeline render
  (`mixer/render/strategy.rs`). On an external-instrument track whose
  playback source is Recorded, the monitor is muted where a recorded take
  covers the block, unless the track is armed (`recorded_monitor_gate`).
- **Ring pacing:** the capture callback pushes whole frames into a monitor
  ring; `MonitorDrain` decides how much the output callback reads and skips,
  in whole frames, with one quantum of jitter margin. A short read while
  monitoring is counted as a monitor-ring shortfall in the load report.
- **Latency:** monitored input is rendered at the raw playhead, so it is
  *not* delayed by plugin-delay compensation, while the backing mix is.
  Any latency-adding plugin (on the monitored track, a bus or the master)
  makes the performer play against a late mix. Recording compensates for
  that when it places the take, but what the performer hears is not
  compensated. That is why the studio skill keeps latency-adding plugins
  (mastering chain, IR) off while tracking. There is no low-latency or
  direct-monitoring mode.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Track fields, undo | `resonance-app/src/update/track.rs` | `TrackMessage::{SetTrackInputDevice, SetTrackInputPort, ToggleTrackMono, ToggleMonitor, ToggleRecordArm}` |
| Inspector pickers | `resonance-app/src/view/mixer/inspector/io.rs` | `audio_input_block` |
| Arm / monitor buttons | `resonance-app/src/view/controls.rs` | `record_arm_button`, `monitor_button` |
| Stream reconcile | `resonance-audio/src/engine/tracks.rs` | `sync_input_stream`, `handle_set_track_monitor`, `handle_set_track_input_device` |
| Stream build | `resonance-audio/src/platform.rs`, `input_pipewire.rs` | `build_input_stream`, `MonitorResampler` |
| Monitor mix | `resonance-audio/src/mixer/monitor.rs`, `mixer/callback/monitor_input.rs` | `MonitorDrain`, `mix_monitor_passthrough`, `read_monitor_input` |

The capture callbacks run on realtime threads: they only push whole frames
into lock-free rings and never allocate.

## Surfaces

- **GUI:** Mix tab strip and Arrange track header: arm (circle) and monitor
  (ear) buttons; Mix inspector → ROUTING → INPUT DEVICE / INPUT CHANNEL;
  Mix inspector → TRACK → Mono. Palette commands "Toggle Input Monitor
  (Selected Track)" / "Toggle Mono (Selected Track)". A frozen track shows
  locked arm and monitor buttons.
- **Control API / MCP:** none for plain audio tracks: an agent cannot set an
  audio track's input, mono, monitor or arm. External-instrument tracks have
  `external.set_return`, `external.set_monitor`, `external.set_record_arm`.
  See [record-control-api](../../intents/hardware-io/record-control-api.md).

## How it's tested

| What | Where | Run |
|---|---|---|
| Stopped / count-in monitor goes through bus, sends, master | `resonance-audio/tests/mixer/stopped_mix_stage.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Whole-frame ring alignment | `resonance-audio/tests/mixer/monitor_ring_alignment.rs`, `recording_whole_frame_push.rs` | same |
| Recorded-source monitor gate | `resonance-audio/tests/mixer/recorded_monitor_gating.rs` | same |
| cpal rate-mismatch resampler | `resonance-audio/tests/mixer/monitor_fallback_resample.rs` | same |
| Channel candidates | `resonance-audio/tests/types/input_channel_candidates.rs` | same |

Not covered: opening a real capture stream, the device-switch behaviour of
`sync_input_stream`, and the app-side input pickers (no golden of the
ROUTING input block).

## Decisions and deviations

- The stopped path used to sum monitored input straight to the output with
  no bus or master chain (code review RT-14); it now runs the full mix.
- The mixer UX treats monitoring as one source at a time; the engine does
  not support two different input devices at once.

## Known gaps

- One capture device per session, device switch while monitoring, no manual
  record offset: [audio-input-routing](../../intents/hardware-io/audio-input-routing.md).
- No control methods for audio-track input/arm/monitor:
  [record-control-api](../../intents/hardware-io/record-control-api.md).
