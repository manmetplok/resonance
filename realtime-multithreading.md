# Realtime multithreading — parallel graph rendering

Status: **built**, P0–P4, 2026-09-28, branch `rt-multithreading`. §11 records
what was built where it differs from the proposal below, the measurements,
and the answers to §9. The live ten-minute stress run (§6) is still to do by
hand.

## 1. Problem

The mixer renders every track, bus and plugin of a block **on one thread**:
PipeWire's RT data thread runs `mixer::callback::mix_audio`, which walks
`render_core::render_block` serially: the track pass, then the bus pass, then
master. The machine has 8 physical cores and 16 hardware threads (Ryzen 7
9700X, 32 MB L3). 15 of those threads sit idle while the one audio thread
xruns.

The trigger was four NAM amps on a 128-frame quantum (2.667 ms budget at
48 kHz). The amp fix (`3e332e4b`, ~6x cheaper per instance) removed that
case, but the ceiling is unchanged: **the whole project's DSP must fit in
one core's 2.667 ms.** Any heavy chain (amp + cab IR + reverb per track,
a stack of wavetable voices, third-party CLAP plugins) will hit it again.

### Goal

Render independent parts of the block graph concurrently on a pool of RT
worker threads, so the per-block budget is roughly `cores x 2.667 ms`
instead of `1 x 2.667 ms`, **without** changing a single output sample.

### Non-goals

- Threading *inside* a plugin (a 128-frame block is too short to split;
  fork/join overhead eats the gain). A plugin that wants its own threads
  can ask for the CLAP `thread-pool` extension later (§9).
- Pipelining across blocks (rendering block N+1 while block N is played).
  That adds a quantum of latency and is a different design.
- Reducing latency/quantum. This buys CPU headroom, which *enables* a
  64-frame tracking quantum, but does not change it.

## 2. What the current engine already gets right

Parallelism is much cheaper here than in most engines, because earlier
refactors removed the usual blockers:

| Property | Where | Why it matters |
|---|---|---|
| Project state is an immutable, wait-free published graph | `engine/render_graph.rs` (ARCH-02) | Workers read `BlockInputs` concurrently with no locks. |
| Sidechain keys are **one block old** (double-banked taps) | `types/sidechain.rs` module docs | No intra-block track→track dependency. Track order already does not matter. |
| Plugin instances sit behind a per-instance `Mutex`, `try_lock`ed live | `render/strategy.rs` `lock_fx` / `lock_instrument` | A worker locks only the instances of the track it renders. No global lock. |
| Latency comp delay lines have a per-track mutex, "one consumer per comp instance" | `latency.rs` `LatencyComp::apply` | Already safe to call from different threads for different tracks. |
| Track/bus live state (meters, last gains, fades) is per-entity atomics | `Track` / `Bus` runtime | Each is written only by the job that renders that entity. |
| Multi-output fan-out runs inside the parent track's iteration | `render/sub_track.rs` | A parent and its sub-tracks are naturally one job. |
| Busses route only to master or, via aux sends, to *higher-indexed* returns | `render/bus_pass.rs` | The bus DAG is known and acyclic by construction. |
| Nothing on the render path allocates or logs | `render/mod.rs`, ARCH-05 | Holds unchanged on worker threads. |

So the block graph is:

```
          ┌── track job 0 ──┐
          ├── track job 1 ──┤            ┌── bus level 0 ──┐
 inputs ──┼── ...          ─┼─ reduce ──►├── bus level 1 ──┼─ reduce ─► master pass ─► out
          └── track job N ──┘   (ordered)└── ...          ─┘  (ordered)   (serial)
     (fully parallel, no deps)
```

## 3. The hard constraint: bit-identical output

The suite pins output bit for bit: DSP goldens, the bounce/freeze caches,
which are "sample-identical to the unfrozen track" (doc #187), and stems
that sum to the mix. Float addition is not associative. If workers summed
into shared bus/master buffers in whatever order they finished, output would
change from run to run.

**Rule: jobs never write shared sums.** Each track job writes its result into
a private per-job slot. A single-threaded **ordered reduction** then performs
exactly the additions today's serial loop performs, in the same order.
Parallel output is then bit-identical to serial output by construction, and
a thread count of 1 is literally today's engine.

The reduction is cheap: `tracks x frames x (1 + sends)` fused scale-adds.
For 100 tracks at 128 frames that is ~50k multiply-adds, a few µs.

## 4. Design

### 4.1 Split `render_one_track` at the summing point

Today `render_one_track` (`render/track_pass.rs`) does
source → key capture → PDC → meters → `route_post_fader` → aux sends → sub-track
fan-out (which routes each sub-track). Split it into:

- **Job (parallel):** disposition, source (clips / instrument / frozen),
  insert chain, sidechain **capture**, PDC, meters, `set_last_gains`. Output is
  a `TrackJobOut` record in the job's slot:
  - the post-FX, post-PDC, pre-fader stereo buffer;
  - the resolved gain ramps and destination (`TrackOutput` / forced master);
  - flags: `has_audio`, `key_only`, `discard_*`;
  - the same for each sub-track the fan-out produced (buffers + ramps +
    dest), in fan-out order.
- **Reduce (serial, audio thread):** for each track in `tracks` order,
  replay exactly `route_post_fader` → `apply_track_aux_sends` → sub-track
  routes from the record. These are the existing functions, fed from slots
  instead of `scratch.track_buf_*`.

Pre-fader aux sends read the pre-fader buffer and post-fader sends apply
the ramp. Both work because the slot holds the pre-fader signal and the
ramps, which is exactly what `apply_track_aux_sends` reads today.

### 4.2 Scratch becomes per-worker plus per-job

`BlockScratch` (`render/context.rs`) splits three ways:

| Today | Becomes |
|---|---|
| `track_buf_l/r`, `port_scratch`, `note_event_buf`, `fx_dry` | **Per worker**, reused by every job that worker runs in the block. |
| (implicit: the post-route sums) | **Per job slot**: pre-fader buffers for the track and its sub-tracks, sized `max_frames`. Owned by the schedule (§4.4). |
| `data`, `bus_bufs` | **Reducer-only**: the audio thread, after the join. |
| `sidechain: &mut SidechainTaps` | Read bank shared read-only. Write bank split so each tap slot is written only by its source's job (slot index resolved at schedule build). |

`MidiStash` is `&mut` and shared today. It is keyed by instrument and
persists across blocks, so it cannot be per-worker: the job that renders a
track next block may run on a different worker. Give each schedule entry
its **own stash slot**, assigned at schedule build. Only that track's job
touches it, so it becomes a disjoint `&mut` again. The fixed-capacity
semantics (`limits.rs`) are unchanged.

Every one of these is disjoint per job. Disjointness is enforced by
handing each job `&mut` to its own slot, not by locks: an `UnsafeCell` slot
array whose indices are claimed exactly once per block (§4.3), wrapped in one
small audited `unsafe` module. The rest of the render path stays safe code.

### 4.3 The worker pool

- **Size:** `physical_cores - 1` workers plus the audio thread itself, so 7 + 1
  on this machine. The audio thread **participates**: it claims jobs like a
  worker, so an idle or slow pool degrades to serial rendering, never to
  silence. SMT siblings are not used by default (two hot DSP threads on one
  core mostly contend). Override with the `RESONANCE_RENDER_THREADS` env var
  and a config key.
- **Priority:** workers must run `SCHED_FIFO` at the same priority as
  PipeWire's data thread. A normal-priority worker preempted mid-job stalls
  the join and xruns. Acquire RT the way PipeWire does for its own threads
  (its RT module / RTKit; **to verify:** whether the `pipewire` crate
  exposes the thread-utils acquire-RT call, or whether we call RTKit over
  D-Bus). Fall back to `pthread_setschedparam` when `RLIMIT_RTPRIO` allows.
  If RT cannot be acquired, **disable the pool** (threads = 1) and report it
  through `AudioEvent`. A non-RT pool is worse than none.
- **Per-thread setup:** call `resonance_dsp::flush_denormals()` on every worker
  at start. MXCSR is per thread; today only the audio thread sets FTZ/DAZ in
  `mix_audio`, and a worker without it would reintroduce denormal spikes. Also
  set the CLAP audio-thread TLS flag (§4.6).
- **Dispatch:** per block, the audio thread publishes `(epoch, job count,
  BlockInputs pointer)`. Workers spin on the epoch for a bounded window
  after the previous block (tens of µs), then park on a futex. The audio
  thread wakes parked workers with one `FUTEX_WAKE`. Jobs are claimed with
  one `fetch_add` on a shared index into the cost-sorted job list (§4.4).
  The join is a countdown the audio thread spins on, and it keeps
  claiming jobs while it waits.
- **Everything on this path is allocation-free and lock-free**: atomics,
  futex wake, spin. No `std::sync::Mutex`, no channels.
- The pool is created and destroyed by the engine thread, never the audio
  thread, alongside the output stream (`output_pipewire.rs` lifecycle).

### 4.4 The schedule

Built on the **engine thread** whenever the render graph is published
(`RenderGraphSlot` edits), and published alongside it (or inside it):

- One entry per top-level track that can render (sub-tracks ride with their
  parent), each with its slot index, stash slot and tap slot.
- **Cost-sorted, longest first** (longest-processing-time scheduling). Each
  job's cost is an EMA of its measured render time, which the job writes
  into a per-track atomic. With atomic claiming, the heavy amp track starts
  first and the cheap tracks fill in behind it. The makespan is then close
  to `max(heaviest job, total / threads)`.
- The heaviest single job bounds the block: one track with a 3 ms chain
  still xruns. That is inherent to track-level parallelism; the load
  report should name the track (§6).
- Buffers for new slots are allocated here, on the engine thread. The
  replaced schedule retires through `retire::publish` like everything else
  in the graph.

### 4.5 Bus pass

Phase 1 (this spec's MVP): **the bus pass stays serial**, rendered by the
audio thread after the track reduction. Busses are typically few.

Phase 2: level-parallel busses. A bus's inputs are track routes (all done
after the reduction) plus aux sends from **lower-indexed busses**, so the
schedule computes levels: level 0 is busses with no bus-sourced sends in,
level k is fed only by levels < k. Busses within a level run in parallel,
each into its own slot, followed by an ordered reduction into master and
into the next level's return buffers. The ordering argument is the same as
in §3.

The master pass (`callback/master_pass.rs`) stays serial.

### 4.6 CLAP threading contract

- CLAP defines `[audio-thread]` as a *role*: the host may call a plugin's
  `process()` from different threads over time, provided the calls are
  never concurrent. The per-instance mutex already guarantees that.
- Implement the host `thread-check` extension (not implemented today):
  `is_audio_thread()` returns the TLS flag set on the PipeWire data thread,
  on workers, and on the offline render threads. Without it, plugins that
  assert on thread identity fall back to their own guesses.
- **Escape hatch:** a per-plugin "serial only" flag (catalog metadata, set
  by hand for any third-party plugin found to misbehave). A track whose
  chain contains such a plugin is claimed only by the audio thread. Our
  own plugins hold no thread-affine state in their process path (checked
  2026-09-27: no `thread_local!` / `ThreadId` in `resonance-plugin`, the CLAP
  host or `plugins/*`).

### 4.7 Offline renders (bounce, stems, freeze, measure)

`RenderStrategy::Bounce` runs the same `render_block` on the render worker's
thread with blocking locks. The same job split works there, and it would
make exports and freezes several times faster. It needs its **own**
normal-priority pool, because an offline render can run while live playback
uses the RT pool (see `RenderStrategy` docs). Phase 3.

## 5. Phasing

Each phase lands on its own and is a no-op at `threads = 1`.

| Phase | Content | Exit criterion |
|---|---|---|
| **P0 Refactor, still serial** | Split `render_one_track` into job + reduce (§4.1). Per-job slots, per-worker scratch, per-entry stash and tap slots (§4.2). Schedule built on the engine thread (§4.4), run by a serial "pool" of 1. | Full suite green with **no golden re-bless**: every existing golden unchanged is the proof the reduction order is right. |
| **P1 RT pool, tracks parallel** | Worker pool (§4.3), RT acquisition and fallback, FTZ/DAZ, TLS flag, CLAP `thread-check`, atomic claiming, cost EMA. | Parallel vs serial bit-identity test (§7) across thread counts. Stress target met (§6). |
| **P2 Bus levels** | §4.5 level-parallel busses. | Same bit-identity test with bus-heavy fixtures. |
| **P3 Offline pool** | §4.7 for bounce, stems, freeze and measure. | Bounce/freeze goldens unchanged. Wall-clock export speedup reported. |
| **P4 (optional)** | CLAP `thread-pool` host extension, serving plugins from the same workers. | Only if a real plugin asks for it. |

P0 is the risky refactor. It touches the core render path, and no
concurrency is involved yet, which makes it much easier to get right first.

## 6. Observability

`cycle_load.rs` measures the whole `mix_audio` call. Add:

- per-block **critical path**: longest job in µs, and which track;
- **pool efficiency**: sum of job time / (wall time x threads);
- **join wait**: time the audio thread spun after running out of jobs.
  High join wait with low efficiency means one job dominates, so name it.
- worker **RT status** and the effective thread count, surfaced to the app
  once at startup and on change.

These go into the existing `CycleReportSlot` seqlock. The engine thread
formats them, as it does today; the audio path never formats.

**Stress target (P1 exit):** a project of 8 tracks, each with NAM amp +
cab IR + reverb, plus 4 wavetable instrument tracks, renders at quantum 128
with zero over-budget cycles for 10 minutes, where the serial engine xruns.

## 7. Testing

- **Bit-identity, the core invariant:** a `resonance-audio` test (group
  `mixer`, not a new binary) renders fixture projects through `render_block`
  at 1, 2, 3 and 8 threads and asserts byte-equal output against the serial
  render. Fixtures should cover sends, sidechains keyed from muted tracks,
  multi-output instruments with sub-tracks, PDC, frozen tracks, and a loop
  seam. Run many iterations with randomized job-claim order (a test hook
  that shuffles the claim sequence), so correctness never depends on
  scheduling luck.
- **Existing goldens unchanged** at every phase. A needed re-bless is a bug.
- **RT safety:** extend the allocation-counting harness to cover the pool's
  dispatch and join across many blocks.
- **Fallback:** with RT acquisition forced to fail, the engine runs at
  `threads = 1` and emits the event.
- **Teardown:** pool drop while playing, device change, and engine restart
  must not leak or hang workers. Use a watchdog, as the plugin-editor tests
  do.
- Hermetic tests (`Resonance::new_for_test`) default to `threads = 1`. The
  parallel path is exercised explicitly by the tests above.

## 8. Risks

| Risk | Mitigation |
|---|---|
| Worker preempted mid-job (not RT, or starved by another RT thread) → join stalls → xrun | RT required, else pool disabled. The audio thread keeps claiming. The join-wait metric makes it visible. |
| A third-party plugin assumes one fixed audio thread | CLAP `thread-check`; per-plugin serial-only flag (§4.6). |
| Wake-up latency eats the gain at small quanta | Bounded spin before parking. The audio thread runs jobs immediately. Measure at 64 frames. |
| Memory bandwidth / L3 contention between heavy jobs | 32 MB L3 is ample for this workload. Measure the scaling curve in P1 before tuning. |
| Reduction-order bug silently changes the mix | P0 lands serial with an unchanged-goldens requirement. The P1 bit-identity test randomizes claim order. |
| One heavy track still bounds the block | Inherent to track-level parallelism. Report the critical-path track so the user can freeze it (freeze exists). |
| Power/thermal: spinning workers | Spin only in a short window after each block, then park. At idle (transport stopped, nothing monitored), skip dispatch entirely. |

## 9. Open questions

1. RT acquisition: PipeWire thread-utils through the `pipewire` crate, or
   RTKit over D-Bus directly? (Verify what the crate exposes.)
2. Default thread count: physical cores − 1, or leave one more core free
   for the GUI (iced) and the PipeWire graph's other clients?
3. Should the schedule live inside `RenderGraph` or next to it? Inside is
   simpler to keep consistent; next to it avoids rebuilding the graph on
   cost-EMA updates. Cost updates are atomics, so inside is likely fine.
4. Is a CLAP `thread-pool` host extension (P4) worth it for any plugin we
   host? Survey installed third-party plugins before building it.

## 10. Where to start

`mixer/render_core.rs` (`render_block`, the phase order) →
`mixer/render/track_pass.rs` (`render_one_track`, the split point) →
`mixer/render/context.rs` (`BlockScratch`, the split) →
`mixer/render/routing.rs` (the functions the reducer replays) →
`types/sidechain.rs`, `mixer/midi_stash.rs` (per-slot ownership) →
`engine/render_graph.rs` (where the schedule is built and published) →
`mixer/callback/mod.rs` and `output_pipewire.rs` (thread and lifecycle).

## 11. As built

Every phase landed as its own commit on `rt-multithreading`, each with the
full suite green and **no golden re-blessed**:

| Phase | Commit | Notes |
|---|---|---|
| P0 | `07155c92` | Job + ordered reduction, still serial. |
| P1 | `a88d2ccd` | RT pool, thread-check, cost EMA, stats. |
| — | `d572b9d1` | Stress bench (`benches/render_pool.rs`). |
| P2 | `cd579d49` | Level-parallel busses. |
| P3 | `70be65ea` | Offline renders on their own pool. |
| P4 | `abf8f458` | CLAP `thread-pool`; thread roles honoured. |

The suite also passes with every live-harness and offline render forced
parallel: `RESONANCE_RENDER_THREADS=4` and `=8`, all goldens unchanged.

### 11.1 Where the build differs from §4

- **Slots are indexed by track-map position** (`mixer/render/slots.rs`), one
  per track *and* sub-track, so a job's set is disjoint by construction: its
  own index plus its sub-tracks'. The main track renders straight into its
  slot; a sub-track's port is copied into its slot after PDC.
- **The live slot pool grows on the engine thread at graph publish**
  (`SlotSupply::ensure`, called by `RenderGraphSlot::publish_locked` before
  the store) and reaches the audio thread over a bounded channel; the
  replaced pool travels back to be dropped engine-side. Offline renderers
  grow their own pool.
- **The job order is built per block on the rendering thread**, not
  published with the graph: `build_schedule` fills a pre-allocated index
  buffer and `sort_unstable`s it by the cost EMA. It is allocation-free and
  O(n log n) for n top-level tracks; publishing it would have meant a
  republish on every cost change (§9 Q3 is therefore moot).
- **The MIDI stash is lent, not split** (§4.2). Before the jobs run, the
  stash moves each instrument's parked events into its slot's `carry`
  (a `Vec` swap); the job delivers from it or parks more into it; the
  reduction swaps it back. `RenderStrategy` no longer holds the stash, so it
  is shared by reference.
- **Key-only decisions are made in a serial prologue.** Whether a silenced
  track or bus must render for a sidechain key (`key_consumed`) reads the
  *consumer's* live state — its last gains and bypass fades — which the
  consumer's own job advances. The serial loop read a lower-indexed
  consumer's state after its update and a higher-indexed one's before; the
  prologue reads all of them before. The two differ only in the block where
  a muted key source's consumer finishes fading out, and only in whether the
  source renders once more, key-only, unheard.
- **Busses pull their sends** (§4.5). A naive level-order push would change
  the order sends add into a return — bus 1 fed by bus 0 is level 1, bus 2
  unfed is level 0, and both sending into bus 3 would arrive 2-then-1
  instead of 1-then-2. Each bus job instead gathers its incoming sends from
  lower-indexed busses in index order when it starts, and busses sum into
  master in a final index-order reduction. A send to a *lower*-indexed bus
  is dropped; it never had an audible effect in the serial loop either.
- **Sidechain capture takes `&self`** (`SidechainTaps::capture_shared`,
  `unsafe`): each tap slot's write bank is written only by the one job that
  renders its source, and keys read the other bank.
- **Workers adopt the caller's scheduling class lazily** (§4.3). The first
  parallel run publishes the audio thread's own policy and priority
  (`pthread_getschedparam`, once); each worker applies it
  (`pthread_setschedparam`, reset-on-fork) and skips that one block while
  it does. A failure disables the pool and the engine emits
  `AudioEvent::RenderThreads` with the reason. Workers do not need to know
  PipeWire's priority up front, and follow the audio thread across a
  backend change.
- **Every spin yields.** At equal `SCHED_FIFO` priority a spinning thread
  is never preempted by a same-priority thread queued on its CPU, so a
  caller spinning at the join could starve the worker it waits for.
  Every 64th spin iteration is a `sched_yield`.
- **Thread count**: `RESONANCE_RENDER_THREADS`, else the app setting
  `audio.render_threads` (`EngineOptions::render_threads`), else physical
  cores (sysfs topology) − 1 workers plus the audio thread. The engine
  records the resolved count process-wide for the offline pools; a process
  with no engine (hermetic tests) renders serially unless the env var says
  otherwise.
- **Serial-only plugins**: `PluginSlot::serial_only`, from a built-in list
  (empty) plus `RESONANCE_SERIAL_ONLY_PLUGINS`. Tracks and busses holding
  one run as caller-only jobs, first.

### 11.2 CLAP threading (§4.6, P4)

- `clap.thread-check`: `is_audio_thread` is a per-thread role flag;
  `is_main_thread` is its negation. Render workers hold the role for life.
  The live callback and each offline chunk take it for the render only,
  so a thread that renders and then makes main-thread calls is never
  misreported.
- **The host's own `[audio-thread]` calls take the role too**:
  `start_processing`, `stop_processing`, `reset`, an active plugin's
  `params.flush`, and `process`. This was found the hard way: with
  thread-check served, u-he Hive checked `start_processing` — which the
  host makes from the engine thread at instantiation — and aborted the
  process. `tests/clap_host/clap_thread_roles.rs` pins every role.
- `clap.thread-pool`: the installed-plugin survey (§9 Q4) found u-he Hive
  and MFM2 both ask for it. `request_exec` becomes a sub-task batch on the
  pool the calling thread renders for: the requester runs tasks, idle
  workers and a caller waiting at its join help, and a second concurrent
  requester (or a thread with no pool) runs its tasks inline — CLAP only
  requires that all ran. Refused outside `process()`.

### 11.3 Measurements

`cargo bench -p resonance-audio --bench render_pool` (release bundles, a
`.nam` model and a cab IR; 9700X, q128 @ 48 kHz, budget 2667 µs; the bench
thread is not realtime, so the max column carries scheduling noise):

| Project | Threads | Mean µs | p99 µs | Over budget |
|---|---|---|---|---|
| 8 × amp+IR+reverb, 4 × wavetable | 1 | 982 | 1384 | 0 / 2000 |
| | 8 | 150 | 191 | 0 |
| 28 × amp+IR+reverb, 4 × wavetable | 1 | 3434 | 3967 | **2000 / 2000** |
| | 2 | 1714 | 1941 | 0 |
| | 4 | 870 | 993 | 0 |
| | 8 | 494 | 582 | 1 |

After the amp's ~6× speedup the spec's 8-guitar project no longer
overloads the serial engine, so the "serial xruns, parallel does not" half
of the P1 exit was shown at 28 guitars. Pool efficiency stays 88–100 %.

Offline master-stem export, 16 guitars + 4 synths, 5.3 s of audio: 1.7×
realtime serial, 3.4× on 2 threads, 6.6× on 4, 11.1× on 8 (6.7× speedup).

Hive, 3 instances with chords: 260 → 94 µs per block on 4 threads.

### 11.4 §9, answered

1. **RT acquisition**: the `pipewire` crate does not expose thread-utils.
   Workers use `pthread_setschedparam`, which works wherever
   `RLIMIT_RTPRIO` allows (98 on this machine). RTKit over D-Bus is **not**
   implemented; without an rtprio limit the pool falls back to serial and
   says so. Open if that fallback turns out to matter.
2. **Default thread count**: physical cores − 1 workers plus the audio
   thread (7 + 1 here). Nothing measured argues for leaving another core
   free; the setting is there if the GUI ever stutters under load.
3. **Schedule location**: neither — built per block on the rendering
   thread (§11.1).
4. **CLAP `thread-pool`**: yes — Hive and MFM2 use it (§11.2).

### 11.5 Still open

- The live ten-minute run of the stress project at q128 with
  `RESONANCE_AUDIO_STATS=1` (§6), which the offline bench cannot replace:
  it is the only check of RT scheduling jitter and wake latency.
- The same at the 64-frame tracking quantum (§8, wake-up latency).
- RTKit, if a machine without an rtprio limit needs the pool.
- A UI surface for `AudioEvent::RenderThreads` and the per-cycle critical
  track (the engine logs both today).

