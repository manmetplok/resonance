# `meter.*` — measuring the mix over the control API

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: ba doc #273 (todos #1218–#1220, #1243); `docs/history/specs/warmth-width-depth.md` §7.1–§7.3, §7.6.

An agent cannot hear, so it reads numbers. The `meter.*` methods put the
app's own BS.1770 / EBU R128 measurements on the wire without writing a
file: loudness, peaks, dynamics, stereo and tonal balance of the master, a
track, a bus or a loaded reference; a whole-mix balance table in one call;
before/after snapshots with loudness-matched deltas; and the harmonic
signature of an insert chain. Nothing here mutates the project.

## Behaviour

### Methods

- **`meter.measure`** — one target over one range.
  - `target`: `"master"` (default: full mix with master FX, volume and
    clip, identical to `render.mixdown`), `{"track_id": N}` (the track plus
    its sub-tracks, before master FX and fader; a bus id is accepted here
    too), `{"bus_id": N}` (everything routed into the bus through its
    chain), or `{"reference": N}` (a loaded reference, measured whole from
    its decoded audio; `meter.measure` only).
  - `range`: defaults to the whole song; a range past the end is clamped.
  - `source`: `"render"` (default; offline render of the target, repeatable)
    or `"live"` (the engine's streaming master meter, "what just played";
    master only, no render, no details, some whole-buffer fields absent —
    check `source` in the result).
  - Result (`MeasureResult`): integrated, max short-term and max momentary
    LUFS (gated, so a part that plays briefly reports how loud it is while
    playing), LRA, true peak (dBTP), sample peak, crest, clipped samples,
    correlation, mono penalty, and `bands` (energy shares of the four AES
    bands, summing to 1).
  - `detail`: any of `spectrum`, `stereo`, `dynamics`, `depth`, `decay`
    adds a block per detail (render source only). Omitted details are
    absent, not `null`.
- **`meter.stems`** — the master plus every top-level track over one shared
  range, in one command and one result (the engine renders each target in
  turn, so cost scales with track count). Sub-tracks get no line of their
  own; they are folded into the parent and listed in
  `includes_track_ids`. `include_busses: true` adds bus lines (which overlap
  their members and must not be summed with them).
- **`meter.snapshot`** — measure and keep the numbers under a
  `snapshot_id`. Snapshots live in app memory for the session, survive
  opening another project, are never saved, and the least recently used is
  evicted past 32 (`SNAPSHOT_CAPACITY`).
- **`meter.compare`** — `a` and `b` are each a `snapshot_id` or
  `"current"` (render now; `b` defaults to current). A snapshot side fixes
  target and range; a current side re-renders exactly that range. With
  `match: "lufs"` (default) B is gain-matched to A's integrated loudness
  first, so level figures move with the match gain and shape figures
  (crest, LRA, correlation, bands, tilt, stereo, PLR/PSR) describe a change
  in character. Deltas are B − A; a delta is `null` when either side lacks
  the number.
- **`meter.probe`** — drive a **clone** of one insert chain (master, a
  track's effects without its instrument, or a bus) with a tone (default
  1 kHz at −18 dBFS; optional SMPTE IMD pair) and report gain, THD,
  harmonics, decay per order, aliasing floor, latency, which stages ran
  and which were skipped. The live instances are never processed: each
  stage's state is copied with one `save_state()` into a fresh instance on
  the engine thread. A probe needs no offline-render guard and can run
  while the transport rolls, but the state copy holds the live slot's lock
  briefly and can cost that plugin one block, as a project save can.

### Jobs and guards

- Measurements run as **jobs** (`job_status` / `job_wait`), like
  `render.mixdown`. The engine answers each `MeasureMix` with exactly one
  `MixMeasured` or `MixMeasureError`, both carrying the job id as
  `measure_id`, so a late event can never complete another caller's job.
- One offline measurement at a time (`OfflineRenderGuard`); a second is
  answered `busy` synchronously. Render measurements are also refused
  while the transport rolls or a WAV bounce, bounce-in-place or freeze is
  running. Live reads are exempt and can run alongside a render
  measurement.
- Dispatched below the mutation gate: with no project open the answer is
  `busy`, not an empty measurement.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Wire types | `resonance-control/src/methods/meter.rs` | `MeasureParams`, `MeasureTarget`, `MeasureSource`, `MeasureDetail`, `MeasureResult`, `StemsParams`, `StemsResult`, `CompareParams`, `CompareDeltas`, `ProbeParams`, `ProbeResult`, `SNAPSHOT_CAPACITY` |
| App handlers | `resonance-app/src/update/control/meter.rs`, `update/control/meter_compare.rs` | `render_guard`, `resolve_range`, `mix_measured`, `mix_measure_error` |
| Jobs, snapshots | `resonance-app/src/control_jobs.rs`, `resonance-app/src/state/control.rs` | `JobBoard`, `JobToken::Measure`, `ComparePlan`, `MeterSnapshots` |
| Engine measurement | `resonance-audio/src/engine/bounce/{measure,stem}.rs` | `render_stem`, `MeasureMix` handling |
| Engine probe | `resonance-audio/src/engine/probe.rs` | `handle_probe_chain` |
| DSP | `resonance-metering` (see [metering-library.md](../plugins-instruments/metering-library.md)) | `LufsMeter`, `TruePeakMeter`, `offline`, `detail`, `probe`, `decay` |
| MCP tools | `resonance-mcp/src/tools/meter.rs` | — |

## Surfaces

- **GUI:** none (the GUI meters are separate; see [metering.md](metering.md)).
- **Control API / MCP:**
  `meter.measure` → `mcp__resonance__meter_measure`,
  `meter.stems` → `mcp__resonance__meter_stems`,
  `meter.snapshot` → `mcp__resonance__meter_snapshot`,
  `meter.compare` → `mcp__resonance__meter_compare`,
  `meter.probe` → `mcp__resonance__meter_probe`.
- **Agent skills:** `mixing`, `mastering`, `spatial` (all five tools, and
  their `references/`).

## How it's tested

| What | Where | Run |
|---|---|---|
| Control methods | `resonance-app/tests/control/{control_meter,control_meter_compare,control_meter_detail,control_meter_probe,control_jobs}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Engine measurement | `resonance-audio/tests/mixer/{measure_mix,measure_reference,stem_render,stem_sub_track_render,stem_bus_sub_track_render}.rs`, `resonance-audio/tests/bounce/{measure_detail,measure_depth}.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Offline guard | `resonance-audio/tests/engine/offline_render_gate.rs`, `resonance-audio/tests/mixer/offline_gate_in_flight.rs` | same |
| Probe | `resonance-audio/tests/clap_host/probe_chain.rs`, `resonance-metering/tests/probe.rs` | `./scripts/run-tests.py -p resonance-audio` / `-p resonance-metering` |

Not covered: no end-to-end test drives `meter.stems` on a large project to
see whether it outlasts the MCP tool's wait.

## Decisions and deviations

- The spec's `plugin.probe` shipped as `meter.probe`.
- `meter.stems` measures each target from its own render inside one pass
  instead of mute-and-bounce, which had let a whole drum kit bleed into
  every stem.
- Probing clones the chain rather than processing live instances, so a
  probe disturbs no compressor state, automation or undo.

## Known gaps

- There is no `job.cancel` and no wall-clock timeout on a measurement job.
  Every engine path emits one terminal event, so jobs do resolve, but if
  one ever did not, every later render measurement would stay `busy` for
  the life of the process. Safe to add since `measure_id` (todo #1243);
  the deadline choice was deferred to `job.cancel` (control-api area).
