# DSP load meter and stutter diagnostics

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/realtime-multithreading.md` §6;
> `docs/history/reviews/field-report-2026-10-06.md` §1. Commits `7461f04c`
> (missed cycles, dropped output, worst lock-miss plugin), `089b951d`
> (stream wake timing), `b6b96244` (workers stop idle-spinning).

The engine measures how much of each audio cycle it uses and records the
stutters a load figure cannot see (late wakeups, missed graph cycles,
silent output cycles, skipped plugins, dropped monitor input). The app shows
a CPU readout; the full picture goes to the log, so a stutter can be traced
to the engine, a plugin, or the system.

## Behaviour

### What is measured (audio thread, atomics only)

- **Load:** each mix call is timed against its budget
  (`frames / sample_rate`). `CycleLoadMeter::record` keeps an EMA
  (`LOAD_EMA_ALPHA` 0.05) and a window peak in `SharedState`
  (`dsp_load_ema_bits`, `dsp_load_peak_bits`). A call that outruns its budget
  is an over-budget cycle (`dsp_overrun_cycles`).
- **Monitor shortfalls:** a short monitor-ring read while monitoring
  (`monitor_shortfall_cycles`): a quantum of live input lost with no xrun
  anywhere.
- **Plugin lock misses:** a live block that skipped a plugin because its
  lock was held (`record_plugin_lock_miss`, plus a per-slot count).
- **Native PipeWire output only** (`output_pipewire::on_process_timed`):
  missed graph cycles (`missed_graph_cycles` from `pw_time.ticks`), dropped
  output cycles (no buffer / no data), wake delay max, late wakes (over 1 ms
  after cycle start, `LATE_WAKE_NS`), longest callback.
- **Render pool:** per window, threads used, the longest single track job and
  its track id (the "critical" track, the one to freeze), efficiency, and
  mean join wait (`PassStats` → `PoolReport`).

### How it is reported

- The mix closure publishes a `CycleLoadReport` through
  `CycleReportSlot` (a seqlock of atomics). The engine loop
  (`load_report::LoadReporter::poll`) formats it and logs at info level,
  appending the plugin with the most lock misses in the window.
- Quiet mode (default): one line per 10 s, only if the window had an
  over-budget cycle, a monitor shortfall, or a peak above 75%.
- `RESONANCE_AUDIO_STATS=1`: one line every 2 s, always.
- Example line:
  `audio: dsp load avg 22.3% peak 177.0% | over-budget cycles 1 (lifetime 4) | monitor shortfalls 0 (lifetime 3) | plugin lock misses 0 (lifetime 42) | render 8 threads, critical 4447 µs (track 26), efficiency 47%, join wait 5.9 µs`.
  Missed graph cycles, dropped output cycles and stream wake figures are
  appended only once non-zero.
- Separate log lines: render-pool status changes (`report_render_pool`),
  cpal underrun summaries (10 s rate limit), stream errors, an oversized
  backend buffer (once).

### What the app shows

`AudioEngine::dsp_load()` → `CpuLoad { smoothed, peak }` → the transport bar's
CPU readout (`view/transport.rs::cpu_readout`), polled every UI tick. None of
the stutter counters reach the UI or the control API.

### Diagnosing a stutter (what the fields mean)

- High peak with over-budget cycles: the mix is too slow. Look at
  `critical` (one heavy track) and `efficiency`.
- Missed graph cycles or late wakes with low load: the callback started late
  (scheduling, IRQ or USB trouble), not DSP. The Scarlett case in the field
  report was SCHED_FIFO workers spinning on the core that services the USB
  softirq; fixed by parking (`LIVE_SPIN_US = 0`).
- Dropped output cycles: PipeWire gave no buffer.
- Monitor shortfalls: input/output ordering flipped on a zero-margin ring;
  `MonitorDrain` locks itself out after the first one
  ([input-capture.md](input-capture.md)).
- Lock misses: something else held a plugin (an editor or engine-thread call
  during `process`); the line names the plugin.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Meter, report, line format | `resonance-audio/src/cycle_load.rs` | `CycleLoadMeter`, `CycleLoadReport`, `PoolReport`, `CycleReportSlot`, `missed_graph_cycles`, `format_cycle_load_line`, `record_plugin_lock_miss`, `OversizeBufferLatch` |
| Engine-side printing | `resonance-audio/src/engine/thread/load_report.rs` | `LoadReporter`, `worst_lock_miss_slot` |
| Per-slot misses | `resonance-audio/src/clap_host/slot_locking.rs` | `try_lock_counted`, `lock_misses` |
| Output-stream counters | `resonance-audio/src/output_pipewire.rs` | `on_process`, `on_process_timed` |
| Counters | `resonance-audio/src/engine/shared_state.rs` | `dsp_*`, `monitor_shortfall_cycles`, `graph_missed_cycles`, `output_dropped_cycles`, `output_wake_max_ns`, `output_callback_max_ns`, `output_late_wakes` |
| App readout | `resonance-app/src/update/tick.rs` (`poll_cpu_load`), `resonance-app/src/view/transport.rs` | `CpuLoad` |

## Surfaces

- **GUI:** CPU readout in the transport bar (smoothed and peak).
- **Control API / MCP:** none.
- **Env:** `RESONANCE_AUDIO_STATS=1`; `RUST_LOG=info` to see the lines.
- **Agent skills:** none (an agent cannot read the log).

## How it's tested

| What | Where | Run |
|---|---|---|
| Load math, quiet gating, shortfall deltas, hand-off, line format, missed-cycle math | `resonance-audio/tests/mixer/cycle_load.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Underrun limiter | `tests/mixer/underrun_rate_limiter.rs` | same |
| CPU readout | `resonance-app/tests/io/project_lifecycle_ux.rs` (via `test_publish_dsp_load`) | `./scripts/run-tests.py -p resonance-app` |

Not covered: the PipeWire timing counters against a real graph, and
`LoadReporter`'s worst-slot naming (no test names it).

## Decisions and deviations

- The audio thread never formats or prints; the engine loop does.
- There is no state-lock miss line any more: the render graph cannot miss
  ([render-graph.md](render-graph.md)).

## Known gaps

- The stutter counters, `RenderThreads` and the critical track are log-only:
  [rt-pool-status-ui](../../intents/audio-engine/rt-pool-status-ui.md).
- No live soak run has validated the counters on a loaded session:
  [rt-live-stress-run](../../intents/audio-engine/rt-live-stress-run.md).
