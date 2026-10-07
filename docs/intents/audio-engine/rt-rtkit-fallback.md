# RTKit fallback for render-pool realtime priority

> Status: **idea** (deferred until a machine needs it)
> Area: audio-engine (`docs/features/audio-engine/render-pool.md`)
> Source: `docs/history/specs/realtime-multithreading.md` §11.4 Q1, §11.5;
> `resonance-audio/src/render_pool/sched.rs` module doc ("RTKit over D-Bus
> is not implemented"); confirmed on master @ `8eac71b3`.

## Why

Render workers get realtime priority with `pthread_setschedparam`, which
needs an `RLIMIT_RTPRIO` (the `audio` group setup in README). On a desktop
without it, PipeWire's own threads still get realtime through RTKit, but the
pool cannot match them, so it falls back to serial rendering and the engine
loses its multi-core headroom. The user only learns this from a log line.

## What it would do

When `pthread_setschedparam` fails with `EPERM`, ask RTKit
(`org.freedesktop.RealtimeKit1.MakeThreadRealtime`) for each worker thread
at the audio thread's priority (within RTKit's limits), and keep the pool
parallel if it succeeds.

## What exists to build on

- `render_pool/sched.rs` (`Sched`, `apply`), `PoolHealth::RealtimeDenied`
  and the `RenderThreads { error }` event.
- The `deny_sched` test hook on `PoolConfig` simulates `EPERM`.

## Open decisions

1. Add a D-Bus dependency to resonance-audio, or a tiny helper crate?
   **Recommended:** a small optional module behind a cargo feature, Linux
   only, so the default build stays lean.
2. RTKit caps priority and requires `RLIMIT_RTTIME`; workers must set it.
   **Recommended:** set `RLIMIT_RTTIME` as RTKit documents before asking.

## Slices

1. Probe: on `EPERM`, try RTKit for one worker; report which path was used
   in `RenderThreads`.
2. Apply to all workers; README note.

## How we'd know it works

On a machine with `ulimit -r` = 0 but RTKit running, the log reports the pool
realtime via RTKit and `parallel_render` figures match the rtprio case.
