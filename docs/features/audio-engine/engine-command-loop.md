# Engine command loop

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/refactor/refactor-intent.md` (epics B, C, D),
> `docs/history/refactor/D-6-engine-created-ids.md`.

The app drives the audio engine by sending it `AudioCommand`s and reading
back `AudioEvent`s. One engine control thread owns all engine state, runs
every command handler, applies worker results, and publishes what the audio
callback reads. Nothing in the app reaches into engine state directly; it
mirrors what the events say.

## Behaviour

### Start-up (`AudioEngine::with_options`)

In order:

1. Probe the default output device (`probe_output`): force the PipeWire
   graph to the canonical 48 kHz where possible (`GraphRateForce`, restored
   on drop), read `clock.quantum` / `clock.force-quantum` and
   `clock.max-quantum`. Fallbacks: quantum 1024, max quantum 2048.
   `buf_frames = max(max_quantum, 2 × quantum, 256)` sizes every scratch
   buffer.
2. Open the channels (`EngineChannels::open`): unbounded command and event
   channels; bounded live-MIDI (1024), live-control (1024) and MIDI-clock
   (4096) channels.
3. Create the wait-free snapshots (`Snapshots::new`): `SharedState` (which
   holds the render graph), tempo map, latency-compensation table,
   automation snapshot.
4. Size the live render pool (`PoolConfig::live`) and record the thread
   count process-wide (`render_pool::configure_threads`). See
   [render-pool.md](render-pool.md).
5. Build the mix callback (`MixerFactory::make`) and open the output
   backend: native PipeWire first, cpal as fallback. See
   [audio-output-stream.md](audio-output-stream.md).
6. Spawn the `resonance-engine` thread (`engine::thread::engine_thread`).

`EngineOptions { render_threads }` is the only start-up option. The app
fills it from `settings.audio.render_threads`. A failure is an
`EngineInitError`; the app shows a native dialog and exits (there is no
headless mode).

The engine thread sends `SampleRateDetected` first. The engine creates no
default track; the app sends `AddTrack` for "Track 1" itself
(`tests/engine/startup_no_default_track.rs`).

### The loop

`engine_thread` marks itself the CLAP main thread, then loops:

- `select!` on the command channel and `SharedState::inbox` (worker results,
  `EngineInternal`), with a 16 ms timeout so polling still runs when idle.
- A command goes through `dispatch::dispatch`, which routes every
  `AudioCommand` variant to one per-category sub-dispatcher
  (`thread/dispatch/{transport,clips,tracks,plugins,bounce,midi,busses,audition,midi_map,reference,peaks,takes}.rs`).
  If `plugins::affects_latency(&cmd)` is true, the PDC table is rebuilt
  afterwards.
- Then, every iteration: drain the inbox, poll plugin host requests (at most
  every 12 ms), live MIDI and controller events, MIDI clock, outbound MIDI,
  count-in, realtime bounce, latency ping, deferred clip commands, tempo
  sync at the playhead, the retire-queue sweep, recording capture and write
  errors, the DSP-load report, render-pool status, stream-error latches,
  plugins that died after an offline render, audition, loop-record seams,
  `PlayheadMoved` (every 16 ms while playing) and `AutomatedValue` events.
- `ShutDown` (or a disconnected channel) breaks the loop. The thread then
  releases every plugin (2 s budget) and drains the inbox.

### Rules handlers follow

- Handlers are free functions taking `&HandlerCtx` (shared state, channels,
  rates) and `&mut HandlerState` (engine-thread-only state: id grant,
  recording, bundles, import queue, take groups, cancel tokens, …).
- A handler that needs a plugin instance `try_lock`s it. If the audio thread
  holds it, the handler re-sends the command on `cmd_tx_retry` instead of
  blocking (`engine/plugins.rs`).
- Heavy work runs on a worker (clip load, import, offline renders, pitch
  analysis, input enumeration). The worker posts its result to the inbox,
  and the engine thread applies it, so only the engine thread edits the
  render graph (`engine/internal.rs`).
- Offline-render and import workers run under `supervise::run_supervised`, so
  a panic still produces the path's terminal error event.

### Ids

The app allocates every track, bus, plugin, send and asset id. The engine
refuses an add whose id is already live with an `AudioEvent::Error` of kind
`Internal`, and never renumbers. Clips the engine has to create at
moments the app cannot predict (record start, loop passes, live-MIDI
capture, realtime bounce) draw from a standing grant: `AudioCommand::GrantIds`
appends ranges, the engine draws in order (`id_grant::ClipIdGrant`), sends
`IdGrantLow` below the low-water mark, and `ClearAll` revokes the grant.

### Handle API

`AudioEngine::send` returns `Err(EngineSendError(cmd))` once the engine
thread is gone; the first disconnect is logged once. `try_recv` is
non-blocking. Read-only accessors read `SharedState` atomics without a round
trip: `is_disconnected`, `output_stream_lost`, `dsp_load`. `shutdown(timeout)`
sends `Stop`, waits for `Stopped`, sends `ShutDown` and joins under a
watchdog; `Drop` calls it with 500 ms.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Handle, start-up, backend choice | `resonance-audio/src/engine/audio_engine.rs` | `AudioEngine`, `EngineOptions`, `EngineInitError`, `EngineSendError`, `probe_output`, `MixerFactory`, `open_output_backend` |
| Engine thread + loop | `resonance-audio/src/engine/thread/mod.rs` | `engine_thread`, `HandlerCtx`, `HandlerState`, `report_render_pool` |
| Command routing | `resonance-audio/src/engine/thread/dispatch/mod.rs` | `dispatch` |
| Worker → engine messages | `resonance-audio/src/engine/internal.rs` | `EngineInternal`, `dispatch_internal`, `drain_internal` |
| Shared state | `resonance-audio/src/engine/shared_state.rs` | `SharedState` (atomics, render graph, retire queue, tables) |
| Wire types | `resonance-audio/src/types/commands.rs`, `types/events.rs` | `AudioCommand` (~160 variants), `AudioEvent` (~160 variants), `IdGrantBlocks` |
| Clip-id grant | `resonance-audio/src/engine/id_grant.rs` | `ClipIdGrant` |
| Worker panic supervision | `resonance-audio/src/supervise.rs` | `run_supervised` |
| App side | `resonance-app/src/lib.rs` (`Resonance::new`), `resonance-app/src/update/tick.rs` (`handle_tick`), `resonance-app/src/engine_events/dispatch.rs` (`handle_engine_event`) | events drained once per UI tick |

Invariants:
- Only the engine thread publishes the render graph (see
  [render-graph.md](render-graph.md)).
- The engine thread never blocks on a plugin lock the audio thread may hold.
- Every `AudioCommand` has an app caller unless it is on the orphan list in
  `tools/arch-invariants`.

## Surfaces

- **GUI:** none directly. Every GUI and control action becomes an
  `AudioCommand`. Engine health shows as a banner ("engine stopped
  responding", "output lost") through `poll_engine_health` in
  `update/tick.rs`.
- **Control API / MCP:** none of its own. Control methods reach the engine
  through the app.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Real handlers without a device or thread | `EngineHandlerHarness` (`resonance-audio/src/engine/thread/test_support.rs`), used across `resonance-audio/tests/engine/*.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Duplicate-id refusal | `tests/engine/track_id_duplicate_rejected.rs`, `bus_id_duplicate_rejected.rs`, `aux_send_id_duplicate_rejected.rs`, `tests/clap_host/plugin_id_duplicate_rejected.rs` | same |
| Clip-id grant | `tests/engine/clip_id_grant.rs`, `create_midi_clip_mandatory_id.rs`, `derived_clip_id_partition.rs` | same |
| Disconnect path | `tests/engine_send_disconnected.rs` (standalone: process-global latch) | same |
| Startup has no default track | `tests/engine/startup_no_default_track.rs` | same |
| App command capture | `Resonance::new_for_test_with_capture()`; `resonance-app/tests/io/id_allocation.rs`, `startup_default_track.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: the real `with_options` start-up (it needs an audio device),
the 16 ms idle cadence, and the shutdown watchdog timing.

## Decisions and deviations

- Worker results travel on an inbox on `SharedState`, not as engine-internal
  `AudioCommand`s, because every worker already holds the `Arc<SharedState>`
  and the payloads have no place in the public enum. The one exception is
  `AudioCommand::BounceTargetCancelled`, posted by the bounce-in-place
  worker.
- The handle has three getters (`dsp_load`, `output_stream_lost`,
  `is_disconnected`). They read atomics and exist so the UI tick never waits
  on a busy engine thread. ARCHITECTURE.md still says not to add getters.
- `ClearAll` keeps clip ids monotonic across loads (code review STATE-08):
  it does not reset any clip counter.

## Known gaps

- The engine still keeps `next_clip_id` (raised by reservations only) and
  allocates take-group ids itself; `ClearAll` does not close an open live-MIDI
  capture (FU-D7d). See
  [engine-created-ids-completion](../../intents/project-core/finish-app-owned-ids.md).
