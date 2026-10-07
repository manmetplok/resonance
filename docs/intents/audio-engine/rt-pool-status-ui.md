# Show render-pool status, stutter counters and the thread setting in the app

> Status: **idea**
> Area: audio-engine data, ui-shell for the view
> Source: `docs/history/specs/realtime-multithreading.md` §11.5 ("A UI
> surface for `AudioEvent::RenderThreads` and the per-cycle critical
> track"); `resonance-app/src/engine_events/dispatch.rs` ("no UI surface
> yet"); confirmed on master @ `8eac71b3`.

## Why

When playback stutters, the useful facts are only in the log: whether the
pool fell back to serial (no realtime priority), which track is the critical
job (the one to freeze), and whether the cause was the mix (over budget) or
the system (missed graph cycles, late wakes). The transport's CPU readout
shows smoothed and peak load only. The thread-count setting
(`audio.render_threads`) exists only in the settings file. An agent sees none
of it either.

## What it would do

- A status popover on the CPU readout: render threads and whether they are
  realtime (with the fallback reason), the critical track by name with a
  "freeze" action, and window/lifetime counts of over-budget cycles, missed
  graph cycles, late wakes, dropped output cycles, monitor shortfalls and the
  plugin with the most lock misses.
- Settings → Audio: render threads (auto / 1…N), applied on restart —
  specified in
  [settings-unexposed-options.md](../ui-shell/settings-unexposed-options.md);
  this intent only adds the status popover.
- Optionally a read-only control method (`engine.status`) for agents.

## What exists to build on

- `AudioEvent::RenderThreads { workers, effective_threads, realtime, error }`
  (received and only logged).
- `cycle_load::CycleLoadReport` / `PoolReport` (published to
  `SharedState::cycle_report`, today consumed only by the engine loop's
  logger, `engine/thread/load_report.rs`).
- `AudioEngine::dsp_load()` and the transport readout
  (`resonance-app/src/view/transport.rs::cpu_readout`).
- `AudioSettings::render_threads` (`resonance-app/src/settings.rs`).

## Open decisions

1. How the report reaches the app: a new `AudioEvent` per report window, or
   an atomic getter like `dsp_load()`? **Recommended:** an event per report
   window (the engine loop already builds the report there), so no new
   getter is added.
2. Map `critical_track` (a track id) to a name app-side.

## Slices

1. Engine: emit the report as an event; app stores it; popover view with a
   golden.
2. Settings → Audio render-threads control (shared with
   [settings-unexposed-options.md](../ui-shell/settings-unexposed-options.md);
   land it there once).
3. Optional `engine.status` control method + MCP tool.

## How we'd know it works

A test publishes a report and a `RenderThreads` fallback and the popover
golden shows both; the setting round-trips through the settings file.
