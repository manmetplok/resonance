# Audio devices and PipeWire

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none in `docs/history/specs/`. Leads: doc #260 (audio-path latency, findings #11/#13/#21), ba todo #1101 (graph-rate force), epic #198.

Resonance plays through the system's default output and records from the
inputs PipeWire (or ALSA/CoreAudio through cpal) offers. There is no device
or buffer-size page: the engine opens a native PipeWire stream on the
default sink, pins the graph to 48 kHz when the device can do it, sizes its
buffers from the PipeWire quantum it reads at startup, and shows the
graph-reported input/output latency in Settings.

## Behaviour

### Output

- **Backend.** On Linux the engine opens a native `pw_stream` named
  `resonance-output` (`output_pipewire::build`), with `node.latency =
  <quantum>/<rate>` and `RT_PROCESS`. The mixer renders straight into the
  graph buffer, so output buffering is about one quantum. If the native
  stream fails, or `RESONANCE_FORCE_CPAL_OUTPUT=1` is set, it falls back to
  cpal (ALSA via the pipewire-alsa shim). On macOS cpal/CoreAudio is the
  only backend.
- **Device.** Always the default output device. The PipeWire stream uses
  autoconnect, so the session manager moves it live when the default sink
  changes. There is no output-device picker.
- **Sample rate.** At startup the engine *asserts* a rate instead of
  following the graph (`platform::assert_graph_rate`): candidates are
  48 000 (`CANONICAL_RATE`), then the current graph rate
  (`clock.force-rate`, else `clock.rate`), then the default sink's rate;
  the first the device supports wins. It writes `clock.force-rate` with
  `pw-metadata` and reads it back (`GraphRateForce::engage`).
  - It skips the write when the graph already runs at that rate and
    `clock.allowed-rates` lists nothing else (`force_is_redundant`): every
    settings write re-negotiates the driver and can drop out.
  - On exit (`Drop`) it restores whatever force was there before, unless
    another client re-forced the graph since (`force_release_target`).
  - Building an input stream for a *different* source re-asserts the rate
    once per source (`reassert_graph_force`, `needs_reassert`).
  - `RESONANCE_NO_GRAPH_FORCE=1` disables all of this; the engine then
    follows the graph (`pick_sample_rate`: pw-metadata, then pactl, then
    cpal's default).
- **Quantum.** Read once at startup with `pw-metadata`
  (`clock.force-quantum` if non-zero, else `clock.quantum`; default 1024 if
  unreadable) plus `clock.max-quantum` (default 2048). Buffers are sized
  `max(max_quantum, 2 × quantum, 256)` frames. The engine never sets the
  quantum. A quantum change while the app runs is not picked up until
  restart. On this machine PipeWire is pinned to 48 kHz / quantum 128
  (64 is the tracking setting).
- The app learns the engine rate from `AudioEvent::SampleRateDetected` and
  rebuilds its tempo map's bar table at that rate (it starts on a 44.1 kHz
  placeholder).

### Inputs

- **Enumeration** runs on a worker thread (`engine/input_devices.rs`,
  `InputDeviceWorker`) because on macOS the first touch of an input blocks
  on the microphone-permission prompt. Linux lists PipeWire/Pulse sources
  with `pactl list sources` (name, description, channel count); anything
  else falls back to cpal (`platform::enumerate_input_devices`). The OS
  default source is reported too and preselected by record-arm.
- The list is fetched at startup, when a bounce dialog opens, and on the
  external-instrument "Re-scan devices" button. There is no audio hotplug
  poll (MIDI ports are polled, see [midi-hardware-io.md](midi-hardware-io.md)).
- **Input streams** are native PipeWire (`input_pipewire`, node
  `resonance-input`) negotiated at the engine rate, falling back to cpal.
  At most 32 channels (`MAX_INPUT_CHANNELS`); a stream that negotiates
  more falls back to cpal. When the cpal input runs at a different rate,
  the monitor path resamples it (`platform::MonitorResampler`); takes are
  resampled once while they are written. How streams open and which
  tracks they serve: [audio-input-and-monitoring.md](audio-input-and-monitoring.md).

### Latency readout

Settings → General → **Audio** shows input latency, output latency and the
round trip, in samples and ms. The figures are the graph's own
(`pw_time.delay` of each native stream, `platform::pw_delay_to_engine_samples`),
asked for with `AudioCommand::QueryIoLatency` when Settings opens and when
the input list changes, and mirrored in `DeviceState::io_latency`. Input
latency is 0 while no input stream is open; the cpal fallback reports 0.
The same capture + playback figures place recorded takes
([recording.md](recording.md)).

### Load and stutter diagnostics

The transport's CPU readout and the `RESONANCE_AUDIO_STATS=1` log line come
from `cycle_load.rs`; see [cycle-load-diagnostics.md](../audio-engine/cycle-load-diagnostics.md). cpal
under/overrun reports are rate-limited to one line per 10 s
(`stream_errors::UnderrunRateLimiter`).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Rate choice, force, quantum, pactl/pw-metadata | `resonance-audio/src/platform.rs` | `choose_assert_rate`, `GraphRateForce`, `force_is_redundant`, `pipewire_quantum`, `enumerate_input_devices`, `build_input_stream` |
| Startup probe | `resonance-audio/src/engine/audio_engine.rs` | `probe_output`, `OutputProbe`, `OutputBackend` |
| Native streams | `resonance-audio/src/output_pipewire.rs`, `input_pipewire.rs` | `build`, `PipeWireOutputHandle` |
| Input enumeration worker | `resonance-audio/src/engine/input_devices.rs` | `InputDeviceWorker` |
| App state | `resonance-app/src/state/devices.rs`, `state/input_devices.rs` | `DeviceState`, `InputDevices`, `IoLatencyInfo` |
| View | `resonance-app/src/view/settings.rs` | `audio_section` in `view_settings_overlay` |

The force is process-global PipeWire state that outlives the process until
the daemon restarts; keep the restore-on-drop rule if you touch it.

## Surfaces

- **GUI:** Settings (gear) → General → Audio (latency rows). No device,
  rate or buffer controls.
- **Control API / MCP:** none for devices or rate. `external.devices` →
  `mcp__resonance__external_devices` lists audio inputs and MIDI outputs
  (see [external-instruments.md](external-instruments.md)).
- **Environment:** `RESONANCE_FORCE_CPAL_OUTPUT`, `RESONANCE_NO_GRAPH_FORCE`,
  `RESONANCE_AUDIO_STATS`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Rate choice, metadata parsing, redundant-force and release rules | `resonance-audio/tests/mixer/graph_rate_assert.rs` | `./scripts/run-tests.py -p resonance-audio` |
| `pw_time.delay` → samples | `resonance-audio/tests/types/pw_latency_math.rs` | same |
| Input channel candidates | `resonance-audio/tests/types/input_channel_candidates.rs` | same |
| Enumeration worker coalescing | `resonance-audio/tests/engine/input_device_worker.rs` | same |
| cpal monitor resampler | `resonance-audio/tests/mixer/monitor_fallback_resample.rs` | same |
| Underrun rate limiter | `resonance-audio/tests/mixer/underrun_rate_limiter.rs` | same |
| Latency readout in Settings | `resonance-app/tests/io/io_latency_readout.rs`; golden `settings_overlay_io_latency-wgpu.png` | `./scripts/run-tests.py -p resonance-app` |
| Native output smoke (ignored) | `resonance-audio/tests/io/pw_output_smoke.rs` | `cargo test -p resonance-audio --test io pw_output_smoke -- --ignored` |

Not covered: the real `pw-metadata` write/readback/restore and the native
input stream need a live PipeWire daemon and are checked by hand
(`pw-metadata -n settings`, `pw-top`). Nothing tests a quantum change
mid-session.

## Decisions and deviations

- **Assert, don't follow.** A 44.1 kHz stream on an idle graph used to drag
  the whole session to 44.1 kHz; the engine now pulls the graph to 48 kHz.
- **Native output instead of cpal** removed the ALSA shim's extra
  ≥2-period ring (2.7–5.3 ms at 48 kHz / q128) and gave real latency figures.
- Positions in the project are stored in samples, but the project's saved
  `sample_rate` is not read on open; see
  [project-sample-rate-mismatch](../../intents/hardware-io/project-sample-rate-mismatch.md).

## Known gaps

- No output/input device page and no live quantum handling:
  [audio-device-settings](../../intents/hardware-io/audio-device-settings.md).
- Opening a project saved at another rate misplaces content:
  [project-sample-rate-mismatch](../../intents/hardware-io/project-sample-rate-mismatch.md).
