# Offline mix measurement and chain probe

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/warmth-width-depth.md` §7 (details,
> probe, depth), "doc #273" and "ba todo #1218 / #1243" cited in code;
> `docs/history/reviews/field-report-2026-10-06.md` §3 (depth estimate).

Because an agent cannot hear, the engine answers loudness and tone questions
about any slice of the mix by rendering it offline and measuring the buffer,
in milliseconds, with nothing written to disk. It can also measure an audio
file or a loaded reference, and probe an insert chain's harmonic signature
with a test tone. Live meters (VU, the master mix meter) are in
[metering.md](../mixer/metering.md) and [reference-ab-monitoring.md](../mixer/reference-ab-monitoring.md); the agent-facing `meter.*` behaviour is in
[meter-control-api.md](../mixer/meter-control-api.md).

## Behaviour

### `MeasureMix { measure_id, targets, range, source, detail }`

- `targets` are `StemSource`s (track with sub-tracks, bus with its members,
  or master), rendered with the stem renderer over one shared range
  (`None` = the whole project). See [stem-export.md](stem-export.md).
- `source`:
  - `Render`: offline render. Takes the renderer only if nothing else is
    rendering (`OfflineRenderGuard::try_acquire_exclusive`), otherwise
    `MixMeasureError { message: MEASURE_BUSY_MSG }`. Needs a stopped
    transport. The app refuses to start a bounce, freeze or export while a
    measurement holds the renderer.
  - `Live`: reads the live master meter (`SharedState::mix_meter`); master
    only, one target, no whole-buffer figures.
- Answers exactly one `MixMeasured` (one `MixMeasurement` per target, in
  request order) or `MixMeasureError`, both echoing `measure_id`.
- `MixMeasurement` figures: integrated LUFS, short-term and momentary max,
  LRA, true peak (dBTP), sample peak, crest over the whole range, clipped
  sample count, L/R correlation over the range, mono penalty, band shares.
  Crest and correlation are computed over the range, not read from the
  100 ms live meters.
- `DetailSet` (all off by default; render path only), from one shared
  spectral analysis: `spectrum`, `stereo`, `dynamics`, `depth`, `decay`
  (EDT/T20/T30 read off a stop), `assist` (LTAS for the mastering
  assistant).
- **Depth** (`fill_depth`): for every return bus a measured track sends to,
  render the return fed only by its feeders' sends (`render_return_stem`)
  and each feeder dry (`render_dry_track_stem`); return gain =
  `10·log10(E_out / Σ E_dry · send²)`. A sub-track feeder (kit tap) is
  send-only: its parent and siblings stay out of the return render (fixed
  in `8cc428b5`). Each track's DRR estimate follows from its sends and those
  gains.

### `MeasureAudio { measure_id, source, detail }`

Measures decoded audio with no render: a file read exactly as a clip would
be (`ClipSource::open_wav_at_rate`), or a loaded reference track. May run
while the transport rolls. Answers one `MixMeasured` (source `Decoded`) or
`MixMeasureError`.

### `ProbeChain` (insert-chain probe)

Builds a **clone** of one insert chain on the engine thread: for each stage,
`save_state()` from the live instance (re-enqueue if its lock is busy), a
fresh instance from the same bundle, state loaded into it. The clones move
to a worker, a synthetic tone runs through them in chain order, the harmonic
signature is analysed, and the clones go back to the engine thread to be
destroyed. The live chain is never touched and playback is not muted.
Answers `ChainProbed { probe_id, report }` or `ChainProbeError`.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Measurement | `resonance-audio/src/engine/bounce/measure.rs` | `measure_mix`, `measure_mix_detailed`, `measure_mix_spawn`, `measure_audio_spawn`, `measure_rendered_buffer`, `fill_depth`, `MEASURE_BUSY_MSG` |
| Return / dry renders | `resonance-audio/src/engine/bounce/stem.rs` | `render_return_stem`, `render_dry_track_stem` |
| Send filtering | `resonance-audio/src/mixer/render/strategy.rs`, `track_pass.rs` | `SendFilter::{OnlyInto, Dry}`, `measured_return`, `replay_route` |
| Types | `resonance-audio/src/types/measure.rs`, `types/probe.rs` | `MeasureSource`, `AudioMeasureSource`, `DetailSet`, `MixMeasurement`, `DepthDetail`, `ProbeSpec`, `ChainProbeReport` |
| Probe | `resonance-audio/src/engine/probe.rs` | `handle_probe_chain` |
| Meters | `resonance-metering` (BS.1770, `offline`, `detail`) | — |
| App + control | `resonance-app/src/update/control/meter.rs`, `update/control/assist.rs` | `meter.*`, `master.assist` handlers |

## Surfaces

- **GUI:** none.
- **Control API / MCP:** `meter.measure` → `mcp__resonance__meter_measure`,
  `meter.stems` → `mcp__resonance__meter_stems`, `meter.snapshot` →
  `mcp__resonance__meter_snapshot`, `meter.compare` →
  `mcp__resonance__meter_compare`, `meter.probe` →
  `mcp__resonance__meter_probe`, `master.assist` →
  `mcp__resonance__master_assist`. All run as jobs.
- **Agent skills:** `mixing`, `mastering`, `spatial` (measure, stems,
  compare, probe; depth and decay details), `mastering` (assist).

## How it's tested

| What | Where | Run |
|---|---|---|
| Measurement figures, details, depth | `resonance-audio/tests/bounce/measure_detail.rs`, `measure_depth.rs`, `tests/mixer/measure_mix.rs`, `measure_reference.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Sub-track feeders in depth | `tests/mixer/sub_track_aux_sends.rs` | same |
| Probe | `tests/clap_host/probe_chain.rs` | same |
| Reverb decay / depth with the real plugin | `tests/clap_host/decay_reverb.rs`, `depth_reverb.rs` | same |
| Control | `resonance-app/tests/control/control_meter.rs`, `control_meter_compare.rs`, `control_meter_detail.rs`, `control_meter_probe.rs`, `control_master_assist.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: depth on tonal (coherent) material, where Room and Chamber
resonate and send on/off energy differences are unreliable (field report §3
caveat).

## Decisions and deviations

- Measurement renders through the same stem filter as export, so a
  multi-output instrument is measured as the whole kit, without bleed.
- `try_acquire_exclusive` refuses rather than queues; the reverse exclusion
  (no bounce while a measurement runs) is enforced app-side.
- Depth assumes uncorrelated feeders: exact for one feeder, close for a
  mix.

## Known gaps

- `MixMeasureError` and `ChainProbeError` carry no error kind (see
  [engine-errors.md](engine-errors.md)).
