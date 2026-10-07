# Latency and plugin delay compensation

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: no dedicated spec; code cites "doc #260" findings #2, #4,
> #9, #10, #13, #19, #20 and code-review findings RT-01, RT-04, MIX-04.
> Memory: epic #198 (audio-path latency).

Plugins that look ahead (limiters, linear-phase EQs, some amps) delay their
output. The engine delays everything else to match, so every track, send and
bus still lines up in playback and in renders, and it measures the device's
input and output latency so recorded takes land where the performer heard
them.

## Behaviour

### What a chain contributes

- A plugin's latency is read from `clap.latency` at activation. When a
  plugin calls `clap_host_latency.changed()` (or `request_restart()`), the
  engine deactivates and reactivates it at the next host-request poll,
  re-reads the latency and rebuilds the table
  (`service_host_restart_request`).
- `slot_latency`: a host-bypassed slot (plugin without its own bypass
  param) contributes 0; a plugin bypassed through its own parameter keeps
  its latency, so the table does not move.
- `chain_latencies`: a track's chain is the sum of its effective plugins.
  A frozen track contributes 0 (its cache is already trimmed). FX bypass
  skips the effects but an instrument still counts. Sub-tracks inherit the
  parent instrument's latency unless the parent is frozen.
- External-instrument tracks add their manual round-trip offset
  (`add_external_offsets`), so the mix waits for hardware audio.

### Two-stage compensation

1. **Track stage:** each track is delayed by `max_track_chain − own` right
   after its chain and before fader, routing and send taps.
2. **Bus stage:** each bus is delayed post-chain by `max_bus_chain − own`;
   everything routed straight to master goes through one shared dry delay of
   `max_bus_chain` (`LatencyComp::apply_dry`), so dry paths meet wet returns.

Total pipeline latency is `max_track_chain + max_bus_chain`. The master chain
is not compensated (it delays everything equally), but its latency is
published as `master_latency_samples` for the reference A/B monitor and for
recording placement.

### Publishing and changes

- `refresh_latency_comp` runs on the engine thread after any command for
  which `affects_latency` is true (plugin add/remove/move, scan, routing,
  track/bus add/remove, `ClearAll`, external-instrument settings) and after
  a plugin restart. If the delays are unchanged nothing is published.
- A new table is built with `LatencyComp::following`: unchanged delay lines
  keep playing; changed ones crossfade to the new alignment over the bypass
  fade length. The old table is retired, not dropped on the audio thread.
- Delays are clamped at `MAX_COMP_LATENCY` (960 000 samples). Hitting the
  clamp sends one `Error` (kind `Plugin`) per engagement.
- Fader, pan and mute automation is evaluated at the comp-delayed position
  (track stage for tracks, full pipeline for busses), so a drawn move lands
  on the audio it was drawn against. Plugin-param automation and MIDI keep
  the raw position, since they act before the delay (`EvalPositions`).
- Delay lines carry across a loop wrap (`continue_across_loop_wrap`).

### Offline renders

Each render builds its own table (`build_latency_comp`), renders
`max_latency` extra frames and drops that many from the front, so a bounce,
export, stem or freeze cache lands exactly on the timeline.

### Device latency

- The native output publishes playback latency each cycle from
  `pw_stream_get_time_n`; the PipeWire input publishes capture latency.
  `pw_delay_to_engine_samples` converts graph ticks to engine samples
  (negative delays clamp to 0).
- `QueryIoLatency` → `IoLatencyReport { capture_samples, playback_samples,
  round_trip_samples }` (capture is 0 when no input stream is open), shown in
  Settings → Audio.
- Record placement subtracts capture + playback latency + the PDC latched at
  record start ([input-capture.md](input-capture.md)).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model + delay lines | `resonance-audio/src/latency.rs` | `slot_latency`, `chain_latencies`, `bus_chain_latencies`, `master_chain_latency`, `add_external_offsets`, `compensation_delays`, `comp_latency_clamped`, `LatencyComp::{new,following,apply,apply_bus,apply_dry,continue_across_loop_wrap}` |
| Rebuild trigger | `resonance-audio/src/engine/plugins.rs` | `affects_latency`, `refresh_latency_comp`, `service_host_restart_request` |
| Restart | `resonance-audio/src/clap_host/state.rs` | `restart` |
| Comp-delayed automation | `resonance-audio/src/mixer/render/context.rs` | `EvalPositions` |
| Offline tables | `resonance-audio/src/engine/bounce/render.rs` | `build_latency_comp` |
| Device latency | `resonance-audio/src/output_pipewire.rs`, `input_pipewire.rs`, `platform.rs` | `pw_delay_to_engine_samples` |

## Surfaces

- **GUI:** Settings → Audio latency readout; the PDC clamp warning banner.
  No per-plugin latency display.
- **Control API / MCP:** none directly. `external_detect_latency` /
  `external_set_latency` feed the external-instrument offset
  ([external-instruments.md](../hardware-io/external-instruments.md)).

## How it's tested

| What | Where | Run |
|---|---|---|
| Comp math and render | `resonance-audio/tests/mixer/latency_comp.rs`, `latency_comp_republish.rs`, `automation_comp_delay.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Latency change from the plugin | `tests/clap_host/clap_latency_tracking.rs` | same |
| Own-bypass keeps latency | `tests/clap_host/own_bypass_latency.rs` | same |
| Bounce trim and master latency | `tests/bounce/bounce_tail_and_master_latency.rs`, `bounce_external_offsets.rs` | same |
| PipeWire delay conversion | `tests/types/pw_latency_math.rs` | same |
| Take placement | `tests/engine/recording_alignment.rs` | same |
| App readout | `resonance-app/tests/io/io_latency_readout.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: device latency figures from a real graph.

## Decisions and deviations

- Two stages instead of one per-track delay, because a signal can leave a
  track by several paths (main out plus sends) whose downstream chains
  differ.
- The `latency.rs` module doc still lists "latency changes a plugin makes
  after activation" as not compensated. That is stale for plugins that call
  `clap_host_latency.changed()`; it remains true for a plugin that changes
  latency without telling the host.

## Known gaps

- A bus → bus aux send crosses the bus stage twice; its wet signal arrives
  `max_bus_chain` late.
- The metronome click and live-input monitoring are rendered at the raw
  playhead and lead plugin-delayed material by the pipeline latency.
