# Parallel render pool

> Verified against master @ `acac5ea3` on 2026-10-15.
> Spec history: `docs/history/specs/realtime-multithreading.md` (its status
> line names branch `rt-multithreading`; the work is on master).

The engine renders the tracks and busses of one block on several threads at
once, so the per-block budget scales with the number of cores instead of
being one core's 2.67 ms. The output is bit-identical to rendering on one
thread.

## Behaviour

### Thread count

Live pool threads (audio thread included), first match wins:

1. `RESONANCE_RENDER_THREADS=<n>`;
2. `EngineOptions::render_threads` (app setting `audio.render_threads` in the
   settings file; takes effect on next launch; no GUI control);
3. physical cores from sysfs topology (SMT siblings counted once; falls back
   to logical CPUs).

`1` renders serially. The engine records the count process-wide
(`configure_threads`) so offline pools use it too. A process without an
engine (hermetic tests) renders serially unless the env var is set.

### How a block runs (`RenderPool::run`)

The rendering thread is itself a worker. It publishes the job closure,
opens an epoch, unparks as many parked workers as there are jobs, runs its
caller-only jobs, then claims shared jobs with one `fetch_add` each, spins
until all are done, closes the epoch and waits for every worker to leave it.
Everything is allocation-free and lock-free (atomics, futex wake, spinning).
A panicking job still counts as done.

- **Order.** Track jobs are sorted per block by a cost EMA (1/8 per block)
  so the heaviest start first. Tracks or busses that hold a serial-only
  plugin run first, on the caller.
- **Reduction.** Jobs never write shared sums; a serial reduction replays the
  additions in track/bus index order ([mixer-render-core.md](mixer-render-core.md)).
- **Busses** run level by level: level 0 is every bus no bus sends into,
  and a bus's level is the longest chain of enabled bus → bus sends ending
  at it — independent of creation order, so a send into an older bus
  still runs its source first (BUG-04). A bus reads another bus's buffer
  only when that bus is at a lower level, i.e. already finished.

### Scheduling

- Live workers adopt the audio thread's scheduling class lazily: the first
  parallel run publishes the caller's `pthread_getschedparam`, each worker
  applies it (`pthread_setschedparam`) and skips that one block.
- If a worker cannot get realtime priority (`RLIMIT_RTPRIO`), the pool falls
  back to serial and the engine emits `AudioEvent::RenderThreads { error }`.
  RTKit is not used.
- **Idle:** live workers park immediately after a block (`LIVE_SPIN_US = 0`,
  commit `b6b96244`). A `SCHED_FIFO` thread spinning on a core starved the
  USB softirq on a Scarlett 18i20 and made the driver xrun every few seconds
  at under 10% load. `RESONANCE_RENDER_SPIN_US` overrides it. Offline pools,
  at normal priority, spin 50 µs (`DEFAULT_SPIN_US`).
- Every 64th spin iteration yields, so a caller spinning at the join cannot
  starve a same-priority worker on its own core.

### Offline pools

Each offline render (export, stems, bounce, freeze, measure) builds its own
pool (`PoolConfig::offline`, normal priority), because it can run while the
live callback owns the realtime one.

### CLAP threading

- Workers hold the CLAP audio-thread role for life; the live callback and
  offline chunks take it per render (see [clap-hosting.md](clap-hosting.md)).
- `clap.thread-pool`: a plugin's `request_exec` becomes a sub-task batch on
  the pool the calling thread renders for; idle workers and a caller waiting
  at its join help. A second concurrent requester, or a thread with no pool,
  runs its tasks inline. Refused outside `process()`.

### Serial-only plugins

`PluginSlot::serial_only` comes from a built-in list (empty) plus
`RESONANCE_SERIAL_ONLY_PLUGINS` (comma-separated CLAP ids). Owners holding
one run as caller-only jobs.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Pool | `resonance-audio/src/render_pool/mod.rs` | `RenderPool`, `PoolConfig::{live,offline}`, `run`, `worker_main`, `configure_threads`, `configured_threads`, `PoolStatus`, `PoolHealth`, `exec_plugin_tasks` |
| Scheduling | `resonance-audio/src/render_pool/sched.rs` | `Sched`, `current`, `apply`, `physical_cores`, `LIVE_SPIN_US`, `DEFAULT_SPIN_US` |
| Jobs + schedule | `resonance-audio/src/mixer/render/track_pass.rs`, `bus_pass.rs`, `slots.rs` | `build_schedule`, `SlotCells`, `PassStats` |
| CLAP thread-pool host side | `resonance-audio/src/clap_host/thread_pool.rs` | `request_exec` |
| Serial-only list | `resonance-audio/src/clap_host/mod.rs` | `serial_only_plugin`, `any_serial_only_plugin` |
| Status report | `resonance-audio/src/engine/thread/mod.rs` | `report_render_pool` → `AudioEvent::RenderThreads` |
| Setting | `resonance-app/src/settings.rs` | `AudioSettings::render_threads` |

## Surfaces

- **GUI:** none. `AudioEvent::RenderThreads` is received and logged at debug
  level (`engine_events/dispatch.rs`); the engine logs the pool status at
  info level.
- **Control API / MCP:** none.
- **Agent skills:** none.
- **Env:** `RESONANCE_RENDER_THREADS`, `RESONANCE_RENDER_SPIN_US`,
  `RESONANCE_SERIAL_ONLY_PLUGINS`, `RESONANCE_AUDIO_STATS` (per-window pool
  figures, see [cycle-load-diagnostics.md](cycle-load-diagnostics.md)).

## How it's tested

| What | Where | Run |
|---|---|---|
| Parallel = serial, bit for bit | `resonance-audio/tests/mixer/parallel_render.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Every golden at 8 threads | whole suite | `RESONANCE_RENDER_THREADS=8 ./scripts/run-tests.py` |
| No allocation in dispatch/join | `tests/render_pool_rt.rs` | `./scripts/run-tests.py -p resonance-audio` |
| CLAP thread-pool | `tests/mixer/plugin_thread_pool.rs` | same |
| Thread roles | `tests/clap_host/clap_thread_roles.rs` | same |
| Slot growth | `tests/mixer/render_slot_supply.rs` | same |
| Stress bench (real plugins; needs `scripts/bundle.sh`) | `resonance-audio/benches/render_pool.rs` | `cargo bench -p resonance-audio --bench render_pool` |

Not covered: realtime wake latency and jitter on the live audio thread. The
bench thread is not realtime. The spec's ten-minute live run at q128 and q64
was never done; see
[rt-live-stress-run](../../intents/audio-engine/rt-live-stress-run.md).

## Decisions and deviations

From the spec (§11.1), confirmed in code:
- slots indexed by track-map position, one per track and sub-track;
- schedule built per block on the rendering thread, not published;
- MIDI stash lent per slot, not split;
- busses pull sends;
- workers copy the caller's class lazily;
- live workers do not idle-spin (changed after the spec's planned 50 µs,
  field report 2026-10-07).

Measured (spec §11.3, 9700X, q128): 28 guitar chains + 4 synths went from
3434 µs mean (every block over budget) serially to 494 µs on 8 threads.

## Known gaps

- No RTKit path: [rt-rtkit-fallback](../../intents/audio-engine/rt-rtkit-fallback.md).
- No UI for pool status, the critical track, or the thread-count setting:
  [rt-pool-status-ui](../../intents/audio-engine/rt-pool-status-ui.md).
- Live stress run: [rt-live-stress-run](../../intents/audio-engine/rt-live-stress-run.md).
