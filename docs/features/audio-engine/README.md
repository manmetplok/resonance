# Audio engine

The `resonance-audio` crate's engine internals: one engine control thread
owns all state and turns the app's `AudioCommand`s into `AudioEvent`s; it
publishes an immutable render graph that the realtime callback (native
PipeWire, or cpal) and the offline renderers read without locks; tracks and
busses render in parallel on a realtime worker pool with bit-identical
output; CLAP plugins are hosted with delay compensation; and the same render
core produces mixdowns, stems, freeze caches and measurements. Mixer,
timeline and plugin *features* built on top of this live in the sibling
areas ([../mixer](../mixer/), [../timeline](../timeline/),
[../plugins-effects](../plugins-effects/), [../plugins-instruments](../plugins-instruments/),
[../control-api](../control-api/)). Open work for this area is in
[../../intents/audio-engine](../../intents/audio-engine/).

| Feature | Doc | One-line summary |
|---|---|---|
| Engine command loop | [engine-command-loop.md](engine-command-loop.md) | `AudioEngine` start-up, the engine thread's command/inbox loop, handler rules, id grants. |
| Engine error taxonomy | [engine-errors.md](engine-errors.md) | `EngineError { kind, message }`, typed export errors, job error kinds, the no-`Result<_, String>` invariant. |
| Render graph publishing | [render-graph.md](render-graph.md) | Immutable `RenderGraph` republished per edit via `ArcSwap`; replaced values freed on the engine thread. |
| Realtime-safety rules | [rt-safety.md](rt-safety.md) | No alloc, lock, log or cold page on the audio path; offline-render gate; limits; how each is enforced. |
| Mixer render core | [mixer-render-core.md](mixer-render-core.md) | The audio callback's branches and `render_block`: track jobs, ordered reduction, bus levels, live vs bounce strategy. |
| Parallel render pool | [render-pool.md](render-pool.md) | Realtime worker pool, thread-count resolution, parked idle workers, CLAP thread-pool, bit-identical output. |
| Audio output stream | [audio-output-stream.md](audio-output-stream.md) | Native PipeWire `pw_stream` output with cpal fallback, graph-rate force, quantum, stream health. |
| Audio input capture | [input-capture.md](input-capture.md) | Capture streams, monitor ring pacing and lockout, streaming takes to disk with latency-correct placement. |
| DSP load and stutter diagnostics | [cycle-load-diagnostics.md](cycle-load-diagnostics.md) | Per-cycle load meter, missed/dropped cycles, wake timing, lock misses, the `RESONANCE_AUDIO_STATS` report. |
| CLAP plugin hosting | [clap-hosting.md](clap-hosting.md) | Scanning, loading, running and removing CLAP plugins; thread roles; host extensions; bypass; editors. |
| Plugin params, state and presets | [clap-params-and-presets.md](clap-params-and-presets.md) | Param description and flush, plugin-made edits, state save/load, preset-state and preset-load, discovery. |
| Latency compensation | [latency-compensation.md](latency-compensation.md) | Two-stage PDC, live latency changes, offline trim, device I/O latency. |
| Offline render and export | [offline-render-and-export.md](offline-render-and-export.md) | Chunked offline render core, mixdown export (formats, normalise, ranged), bounce in place. |
| Stem export | [stem-export.md](stem-export.md) | Track / bus / master stems over one shared range, sequential with partial-failure reporting; the Export dialog and `render.stems`. |
| Offline measurement and chain probe | [offline-measurement.md](offline-measurement.md) | Render-and-measure loudness and details (incl. depth), measure files/references, probe a cloned chain. |
| Track freeze (engine side) | [track-freeze.md](track-freeze.md) | Freeze-cache render, cache format, rate conversion and playback substitution. |
| Clip loading and import | [clip-loading.md](clip-loading.md) | Pool import, mmapped clip loading with tickets and fences, clip WAV persistence. |
