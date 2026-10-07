# Realtime-safety rules

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/realtime-multithreading.md` §7;
> `docs/history/reviews/code-review-todo.md` (RT-*, MIX-*, ARCH-05 findings).

The audio callback and the render-pool workers must finish each block inside
the quantum (2.67 ms at 48 kHz / 128 frames) on every block. These are the
rules the engine follows to make that true, and the mechanisms that enforce
them. Anyone changing code under `mixer/`, `render_pool/`, `clap_host/process.rs`
or the output/input stream callbacks must keep them.

## Behaviour

### What the realtime path may not do

- **Allocate or free.** All scratch is pre-allocated by `MixerFactory::make`
  (track, bus, port, monitor buffers, MIDI event buffer, stash, sidechain
  taps, bypass dry scratch, render slots, A/B meters). Anything the engine
  replaces is retired and freed on the engine thread
  ([render-graph.md](render-graph.md)).
- **Block on a lock.** The project is read through one wait-free
  `ArcSwap::load`. Plugin instances are reached with `try_lock`
  (`PluginSlot::try_lock_counted`); a miss skips that plugin for the block
  (an effect passes dry, an instrument is silent), parks its MIDI in the
  `MidiStash`, and is counted. The engine thread never waits on a plugin
  lock either: it re-enqueues the command.
- **Log or format.** The callback stores counters and latches in atomics;
  the engine loop formats and prints them (`cycle_load::CycleReportSlot`,
  `OversizeBufferLatch`, `StreamErrorLatch`).
- **Touch cold pages.** `prefault::prefault_f32` writes one sample per 4 KB
  page of every scratch buffer at build time. `rt_prep::claim_arc_swap_node`
  gives each realtime thread its `arc-swap` debt node before its first block
  (render workers at spawn; the PipeWire data loop via an invoke queued right
  after `connect`); `seed_arc_swap_nodes` covers cpal's thread, which runs no
  code of ours first.

### Gates and fallbacks

- **Offline render gate.** Any offline render (export, stems, bounce in place,
  freeze, measurement) holds an `OfflineRenderGuard`. While one is held the
  callback outputs silence, holds the transport, and touches no plugin; live
  MIDI is forwarded for bookkeeping only. Raising the gate waits (up to 1 s)
  for a callback already inside a block (`CallbackActivity`, a Dekker
  handshake). Play / Record / realtime bounce / MIDI-clock start refuse with
  `OFFLINE_RENDER_BUSY_MSG` while it is up.
- **Oversized buffer.** If the backend hands the callback more frames than
  the scratch holds (cpal `BufferSize::Default` fallback only), the callback
  renders `buf_frames` and leaves the rest silent; the engine logs it once.
- **Non-finite plugin output.** `process.rs` zeroes NaN / ±Inf at every plugin
  output before it re-enters the graph.
- **Denormals.** `resonance_dsp::flush_denormals()` at the top of every live
  callback and every offline chunk, and on each render-pool worker
  thread.
- **Fixed capacities** (`limits.rs`): `MAX_BUSSES` 32, `MAX_PLUGIN_OUTPUT_PORTS`
  8, `MAX_INPUT_CHANNELS` 32, `MAX_MIDI_EVENTS_PER_BUFFER` 512,
  `MAX_PENDING_PARAMS` 128, `MAX_PENDING_AUTOMATION` 2048, `MAX_PENDING_MIDI` /
  `MAX_PENDING_NOTES` 256, `MAX_STASHED_INSTRUMENTS` 64 × `MAX_STASHED_EVENTS`
  256, `MAX_COMP_LATENCY` 960 000 samples. Each constant documents what
  happens at the limit (drop, clamp or refuse).
- **cpal stream errors.** cpal 0.17 reports underruns through `err_fn`;
  `stream_errors::UnderrunRateLimiter` coalesces them into one line per 10 s.
  Other stream errors log at error level and, on the output, set
  `output_stream_lost`.

### Render-pool specifics

Live workers run `SCHED_FIFO` at the audio thread's priority, park instead
of spinning (`LIVE_SPIN_US = 0`), and yield every 64 spin iterations. See
[render-pool.md](render-pool.md) and
[cycle-load-diagnostics.md](cycle-load-diagnostics.md) for why.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Scratch build + prefault | `resonance-audio/src/engine/audio_engine.rs`, `src/prefault.rs` | `MixerFactory::make`, `prefault_f32` |
| Per-thread arc-swap node | `resonance-audio/src/rt_prep.rs` | `claim_arc_swap_node`, `seed_arc_swap_nodes` |
| Callback entry | `resonance-audio/src/mixer/callback/mod.rs` | `mix_audio`, `block_frames`, `forward_live_midi_unplayed` |
| Offline gate | `resonance-audio/src/engine/bounce/mod.rs` | `OfflineRenderGuard`, `CallbackActivity`, `OFFLINE_RENDER_BUSY_MSG` |
| Plugin lock policy | `resonance-audio/src/clap_host/slot_locking.rs`, `mixer/render/strategy.rs`, `engine/bounce/render.rs` | `try_lock_counted`, `RenderStrategy::{Live,Bounce}`, `try_lock_with_backoff` |
| Output scrub | `resonance-audio/src/clap_host/process.rs` | `scrub_non_finite` |
| Limits | `resonance-audio/src/limits.rs` | constants |
| Stream errors | `resonance-audio/src/stream_errors.rs` | `UnderrunRateLimiter`, `StreamErrorLatch` |

## Surfaces

- **GUI:** the transport's CPU readout and the stream-lost banner are the
  visible results; see [cycle-load-diagnostics.md](cycle-load-diagnostics.md).
- **Control API / MCP:** none.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| No Rust allocation anywhere in the process while the pool dispatches and joins | `resonance-audio/tests/render_pool_rt.rs` (standalone, `#[global_allocator]`) | `./scripts/run-tests.py -p resonance-audio` |
| Render thread frees nothing under heavy edits | `tests/retire_queue/render_graph_hammer.rs` | same |
| Sidechain `begin_block` never allocates | `tests/sidechain_taps.rs` | same |
| Offline gate | `tests/engine/offline_render_gate.rs`, `tests/mixer/offline_gate_in_flight.rs`; app: `resonance-app/tests/io/offline_render_gate.rs` | both crates |
| Lock contention | `tests/clap_host/plugin_lock_contention.rs`, `tests/bounce/bounce_plugin_lock.rs` | same |
| Output scrub | `tests/clap_host/plugin_output_scrub.rs` | same |
| Underrun limiter | `tests/mixer/underrun_rate_limiter.rs` | same |
| Event caps | `tests/mixer/midi_event_cap.rs` | same |
| Bit-identity at any thread count | whole suite with `RESONANCE_RENDER_THREADS=8` | `RESONANCE_RENDER_THREADS=8 ./scripts/run-tests.py` |

Not covered: allocations below Rust's allocator (glibc's one-off TLS
destructor entry on cpal's first callback, see `rt_prep.rs`), page faults in
plugin code or data (the field report 2026-10-06 §1 wavetable case was fixed
in the plugin, not the host), and real scheduling jitter (no live stress run,
see [rt-live-stress-run](../../intents/audio-engine/rt-live-stress-run.md)).

## Decisions and deviations

- The host does not `mlockall`. Field report 2026-10-06 §1 rejected it as the
  primary fix (pins ~1.3 GB, can make allocations fail); the wavetable plugin
  locks its own table bundle instead.
- A plugin lock miss costs the plugin one block rather than blocking the
  audio thread. The miss count names the slot in the load report.

## Known gaps

- Third-party plugins' cold code and data still page-fault on first use.
- The live stress run that would show scheduling jitter has never been done:
  [rt-live-stress-run](../../intents/audio-engine/rt-live-stress-run.md).
