# Live stress run of the parallel render pool

> Status: **designed** (procedure in the spec; never run)
> Area: audio-engine (`docs/features/audio-engine/render-pool.md`)
> Source: `docs/history/specs/realtime-multithreading.md` §6 and §11.5
> ("Still open"); memory note "live 10-min run pending".

## Why

The render pool was proven offline: bit-identity tests, an allocation guard,
and a benchmark whose bench thread is not realtime. None of that measures
realtime wake latency, jitter, or what the pool does to the rest of the
system. The Scarlett stutter (field report 2026-10-07: SCHED_FIFO workers
spinning starved the USB softirq) showed that system-level effects are where
the risk is, and it was found by accident in a session.

## What it would do

A recorded, repeatable live check: the stress project played for ten minutes
at q128 and again at q64 (tracking mode), with `RESONANCE_AUDIO_STATS=1`,
on the USB interface, with a written result: over-budget cycles, missed graph
cycles, late wakes, dropped output cycles, monitor shortfalls, lock misses,
pool efficiency and join wait, and the same figures serial for comparison.

## What exists to build on

- `resonance-audio/benches/render_pool.rs` (the stress project: amp + IR +
  reverb chains and wavetable instances; needs `scripts/bundle.sh`).
- `cycle_load` report fields (`docs/features/audio-engine/cycle-load-diagnostics.md`):
  since `7461f04c` / `089b951d` it reports exactly the counters the run needs.
- `RESONANCE_RENDER_THREADS`, `RESONANCE_RENDER_SPIN_US`.
- PipeWire quantum switch: `pw-metadata 0 clock.force-quantum 64` (the
  engine reads the quantum at start-up, so restart the app after switching).

## Open decisions

1. Load the stress project through the app (a saved project) or a dedicated
   binary? **Recommended:** a saved project opened in the app, so the real
   output path and GUI load are included.
2. Pass criteria. **Recommended:** zero over-budget and zero missed graph
   cycles at q128 over ten minutes; at q64, report numbers and decide.

## Slices

1. Save the bench's stress project as a `.rproj` (or a script that builds it
   over MCP).
2. Run q128 serial and 8 threads; record figures in a short report under
   `docs/history/reviews/`.
3. Same at q64.

## How we'd know it works

The report exists, with the numbers, and any failure has its own intent.
