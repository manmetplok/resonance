# Audio output stream

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: no spec file; "doc #260" findings #11–#13 are cited in the
> code. Memory note: PipeWire 48 kHz / quantum 128 on this machine.

How the mix reaches the speakers. On Linux the engine opens a native
PipeWire playback stream that renders straight into the graph's cycle
buffer; cpal (ALSA through the pipewire-alsa shim) is the fallback and the
only backend on macOS.

## Behaviour

### Rate and quantum

- At start-up the engine asserts the canonical rate (`CANONICAL_RATE` =
  48 000) on the PipeWire graph through the settings metadata
  `clock.force-rate`, when the device supports it and
  `clock.allowed-rates` does not already pin it (`assert_graph_rate`,
  `GraphRateForce`). The guard restores the previous force on drop.
  `RESONANCE_NO_GRAPH_FORCE` disables this. If the force is rejected, the
  engine follows the graph rate (`pick_sample_rate`: graph rate, then the
  default sink's rate via `pactl`, then cpal's default).
- Opening an input stream on a different source re-asserts the forced rate
  (`reassert_graph_force`).
- The quantum is read once at start-up from `clock.force-quantum`, else
  `clock.quantum` (fallback 1024); the max quantum from `clock.max-quantum`
  (fallback 2048). A quantum change later is not followed.

### Native PipeWire (`output_pipewire::build`)

- A `ThreadLoop` named `resonance-output` with one `pw_stream`:
  `media.role = Production`, `node.latency = <quantum>/<rate>`, F32LE stereo,
  flags `AUTOCONNECT | MAP_BUFFERS | RT_PROCESS`. It targets the default sink
  and follows it when the default changes.
- `on_process` runs on PipeWire's realtime data loop: dequeue the cycle
  buffer, run the mix closure straight into it, queue it back. No
  intermediate ring, so output buffering is about one quantum.
- Each cycle it also publishes:
  - the playback latency from `pw_stream_get_time_n` (`time.delay` in engine
    samples) into `SharedState::playback_latency_samples`;
  - missed graph cycles (the driver clock advanced 1.5+ cycles between two
    callbacks) into `graph_missed_cycles`;
  - the wake delay (cycle start to callback entry) as a window max, and
    callbacks waking more than 1 ms late (`LATE_WAKE_NS`);
  - the longest callback;
  - dropped cycles (no buffer, no mapped data, misaligned) into
    `output_dropped_cycles`.
  The engine loop folds these into the load report
  ([cycle-load-diagnostics.md](cycle-load-diagnostics.md)).
- `on_state_changed` sets `output_stream_lost` when the stream enters the
  error state and clears it when it is streaming again.
- Format negotiation is asynchronous; `build` waits up to 500 ms for the
  first `param_changed` and otherwise uses the requested values.
- Before the first cycle, `prepare_process_thread` queues an invoke on the
  data loop that claims the thread's `arc-swap` node.

### cpal fallback

Used when PipeWire init fails, when `RESONANCE_FORCE_CPAL_OUTPUT` is set, and
always off Linux. It requests `BufferSize::Fixed(quantum)` and falls back to
`BufferSize::Default` (logged as high latency) if the device refuses. The
ALSA shim adds two or more periods of latency. Underruns are rate-limited;
other errors set `output_stream_lost`.

### What the app sees

- `AudioEngine::output_stream_lost()` → `EngineHealth::StreamLost` banner
  (`update/tick.rs::poll_engine_health`).
- `AudioCommand::QueryIoLatency` → `AudioEvent::IoLatencyReport { capture,
  playback, round_trip }`, shown in Settings → Audio (re-queried when the
  device list changes).
- `SampleRateDetected { sample_rate }` once at start-up.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Backend choice | `resonance-audio/src/engine/audio_engine.rs` | `probe_output`, `open_output_backend`, `build_cpal_stream`, `build_cpal_with_fallback` |
| Native stream | `resonance-audio/src/output_pipewire.rs` | `build`, `PipeWireOutputHandle`, `on_process`, `on_process_timed`, `on_state_changed`, `prepare_process_thread` |
| Rate / quantum / graph force | `resonance-audio/src/platform.rs` | `CANONICAL_RATE`, `pipewire_quantum`, `pipewire_max_quantum`, `GraphRateForce`, `assert_graph_rate`, `pw_delay_to_engine_samples` |
| Stream errors | `resonance-audio/src/stream_errors.rs` | `StreamErrorLatch`, `UnderrunRateLimiter` |
| App health | `resonance-app/src/update/tick.rs` | `poll_engine_health` |

## Surfaces

- **GUI:** the "output lost" banner; Settings → Audio latency readout.
- **Control API / MCP:** none.
- **Env:** `RESONANCE_FORCE_CPAL_OUTPUT`, `RESONANCE_NO_GRAPH_FORCE`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Native output smoke (ignored; needs a PipeWire daemon and a sink) | `resonance-audio/tests/io/pw_output_smoke.rs` | `cargo test -p resonance-audio --test io pw_output_smoke -- --ignored` |
| Latency math | `tests/types/pw_latency_math.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Graph-rate pure helpers | `tests/mixer/graph_rate_assert.rs` | same |
| Missed-cycle and report formatting | `tests/mixer/cycle_load.rs` | same |
| Underrun limiter | `tests/mixer/underrun_rate_limiter.rs` | same |
| App stream-lost banner / latency readout | `resonance-app/tests/mixer/tick_gating.rs`, `timeline/status_area_keeps_widget_state.rs`, `io/io_latency_readout.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered in the default run: anything that opens a real stream. Hermetic
tests never open one (`Resonance::new_for_test`).

## Decisions and deviations

- Native PipeWire output replaced cpal as the default (doc #260 finding #11)
  to drop the shim's two-period ring and get real latency figures.
- The `pw_output` handle is held only for its `Drop`; its `with_stream`
  accessor has no caller.

## Known gaps

- The quantum is read once at start-up; switching PipeWire between 128 and
  64 (tracking mode) needs an app restart.
- The output device cannot be chosen in the app; the stream follows the
  system default sink.
