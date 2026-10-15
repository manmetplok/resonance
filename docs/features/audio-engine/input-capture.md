# Audio input capture

> Verified against master @ `acac5ea3` on 2026-10-15.
> Spec history: no spec file; code cites "doc #260" findings #2, #3, #12,
> #13 and code-review findings RT-01, RT-02, RT-08, RT-14, RT-17, MIX-09,
> ENG-03.

The engine side of hearing and recording a live input: opening the capture
stream, passing monitored input through track chains, and streaming armed
tracks to WAV files on disk with sample-accurate placement. Recording UX,
take lanes and comping are in [recording.md](../hardware-io/recording.md) and [takes-and-comping.md](../timeline/takes-and-comping.md); external
instruments in [external-instruments.md](../hardware-io/external-instruments.md); per-track input and monitoring in [audio-input-and-monitoring.md](../hardware-io/audio-input-and-monitoring.md).

## Behaviour

### Opening the stream

- One input stream at a time, owned by `RecordingState::input_stream`
  (`InputHandle::{PipeWire, Cpal}`). It is opened on demand: when a track
  turns monitoring on (`sync_input_stream` in `engine/tracks.rs`) or at
  record start (`transport::begin_recording_stream`), from the source the first
  monitored or armed track names. It is rebuilt only if none is open or more
  channels are needed than the open one has.
- `platform::build_input_stream` tries native PipeWire first
  (`input_pipewire::build`), then cpal. PipeWire is preferred because cpal
  through pipewire-alsa downmixes every capture to two channels. Channel
  count is capped at `MAX_INPUT_CHANNELS` (32); a PipeWire stream that
  negotiates more falls back to cpal.
- Opening an input on a new source re-asserts the forced graph rate.
- The PipeWire input publishes its capture latency into
  `SharedState::capture_latency_samples`.
- Device enumeration runs on a worker (`engine/input_devices.rs`), because on
  macOS it can block on the microphone-permission prompt. Requests during a
  pass are coalesced. `ListInputDevices` answers with `InputDevicesListed`.

### Monitoring

- The capture callback pushes whole interleaved frames into a monitor ring
  (`audio_quantum × MAX_INPUT_CHANNELS × 4` samples). Every callback reads
  it (`monitor_input::read_monitor_input`), even while an offline render
  gates the mix, so monitoring resumes at its standing latency.
- Pacing: `monitor_catchup_skip` keeps a one-quantum margin; on the native
  backend `MonitorDrain` drains a sticky extra quantum after 16 stable
  callbacks (`MONITOR_DRAIN_STREAK`). The first shortfall *after* a drain
  locks the drain out for the session, trading one quantum of monitor
  latency for click-free monitoring. A short read while monitoring counts
  as a monitor shortfall in the load report.
- While stopped or counting in, monitored input runs through the track's
  chain, its route and its aux sends, then busses and the master chain
  (`mix_monitor_passthrough`, `callback::idle_mix`). While playing, it is
  mixed inside the render core (`RenderStrategy::mix_monitor`).
- Live monitoring is not delay-compensated; it leads plugin-delayed
  material by the PDC latency.

### Recording

What recording does for the user — count-in, take placement, overflow and
disk-error handling, cycle record, MIDI capture, undo — is documented once,
in [hardware-io/recording.md](../hardware-io/recording.md). Engine-internal
details not repeated there:

- The input callback latches the first pushed frame's position
  (`SharedState::latch_recording_start`) and pushes whole frames into a
  recording ring sized `max(rate × channels × 10 s, 96 kHz stereo × 10 s)`.
  Frames a full ring drops are counted with
  `SharedState::note_recording_dropped` (two atomic adds) and filled with
  silence by the drain (BUG-23).
- `poll_recording_capture` de-interleaves per armed track, resamples to the
  engine rate when the device rate differs (`StreamingLinearResampler`),
  and writes through a `hound::WavWriter` straight into
  `{project_dir}/audio/`. Finalising memory-maps the file and publishes a
  `ClipSource::Mapped` clip.
- Placement subtracts the PDC latched at record start
  (`record_pdc_samples`); a realtime bounce keeps the raw latch and is
  shifted by the external round trip instead.
- Clip ids come from the app's grant
  ([engine-command-loop.md](engine-command-loop.md)).
- Cycle passes are cut by input-frame count at each seam
  (`roll_loop_record_audio_pass` → `RecordingState::roll_audio_pass`);
  take lanes are in
  [timeline/takes-and-comping.md](../timeline/takes-and-comping.md).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Stream builder + fallback | `resonance-audio/src/platform.rs` | `build_input_stream`, `build_input_stream_cpal`, `enumerate_input_devices`, `MonitorResampler`, `input_channel_candidates` |
| Native input | `resonance-audio/src/input_pipewire.rs` | `build`, `PipeWireInputHandle` |
| Handle | `resonance-audio/src/input_handle.rs` | `InputHandle` |
| Enumeration worker | `resonance-audio/src/engine/input_devices.rs` | `InputDeviceWorker` |
| Monitor pacing + pass-through | `resonance-audio/src/mixer/monitor.rs`, `mixer/callback/monitor_input.rs` | `MonitorDrain`, `monitor_catchup_skip`, `monitor_read_len`, `mix_monitor_passthrough` |
| Ring pushes | `resonance-audio/src/mixer/recording_push.rs` | `push_recording_frames`, `whole_frame_push_len` |
| Ring size | `resonance-audio/src/engine/recording_ring.rs` | `recording_ring_len`, `RECORDING_RING_SECONDS` |
| Take writing | `resonance-audio/src/recording.rs` | `RecordingState`, `TrackRecordingBuf`, `drain_ring_to_buffers`, `apply_start_latch` |
| Record start/stop + placement | `resonance-audio/src/engine/transport.rs` | `begin_recording_stream`, `apply_recording_start_latch`, `poll_recording_capture` |

## Surfaces

- **GUI:** track arm / monitor / input device and port in the mixer
  inspector; record transport. See [recording.md](../hardware-io/recording.md), [audio-input-and-monitoring.md](../hardware-io/audio-input-and-monitoring.md) and
  [channel-strips-and-inspector.md](../mixer/channel-strips-and-inspector.md).
- **Control API / MCP:** none for audio recording (an agent cannot record).
  `external.*` tools drive the external-instrument variant
  ([external-instruments.md](../hardware-io/external-instruments.md)).

## How it's tested

| What | Where | Run |
|---|---|---|
| Take placement end to end (fake input) | `resonance-audio/tests/engine/recording_alignment.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Drain, overflow, start latch, whole frames | `tests/mixer/recording_drain.rs`, `recording_overflow_event.rs`, `recording_start_latch.rs`, `recording_whole_frame_push.rs` | same |
| Write failure keeps the take | `tests/recording_write_failure.rs` (standalone: `RLIMIT_FSIZE`) | same |
| Monitor ring | `tests/mixer/monitor_ring_alignment.rs`, `monitor_fallback_resample.rs`, `recorded_monitor_gating.rs` | same |
| Enumeration worker, channel candidates | `tests/engine/input_device_worker.rs`, `tests/types/input_channel_candidates.rs` | same |
| Loop-record takes | `tests/engine/loop_record_takes.rs` | same |

Not covered: a real capture device (no ignored live test for input, unlike
the output smoke test), and the `MonitorDrain` lockout against a real graph.

## Decisions and deviations

- Takes stream to disk from the engine thread, not the audio thread, and
  never exist as one in-RAM buffer.
- Zero-margin monitoring is opportunistic and self-disabling (the
  shortfall lockout) rather than configurable.

## Known gaps

- PipeWire input negotiation that times out (500 ms) keeps the requested
  rate and channel count; the `AudioCommand::InputRateNegotiated` correction
  the module docs mention does not exist.
- Live monitoring and the metronome are not delay-compensated (see
  [latency-compensation.md](latency-compensation.md)).
