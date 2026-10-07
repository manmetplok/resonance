# Granular delay (`com.resonance.granular-delay`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: epic #196 and ba docs #252/#253 (design), #264 (editor); not in `docs/history/`. Audit gaps G1–G3 in `docs/history/reviews/plugin-audit.md` (inert `diffusion`, inert `density_sync`, unreachable WSOLA alignment) were fixed since.

A delay whose buffer is read by a cloud of short, enveloped grains instead
of one tap. Each grain is independently transposed, panned and timed, so
the plugin covers clean echoes, smeared clouds, shimmer that climbs in
octaves, a voice doubler (pitch-synchronous grains), reverse haze and
frozen drones. Pitch can snap to semitones or a scale.

## Behaviour

- **Time** (`sync`, `division`, `time_ms` up to 4000 ms): where the grains
  read, behind the write head. `time_mode` sets what a time change does:
  **Grain** (default; only new grains use the new time, in-flight grains
  finish where they started), **Fade** (the wet fades out, the tap jumps on
  silence, it fades back in) or **Repitch** (the delay slews like tape and
  grains spawned meanwhile take the matching pitch swoop).
- **Grains** (`grain_size_ms` 10–500, `density_hz` 0.5–100, `texture`,
  `spray_ms`, `size_jitter`, `level_jitter`, `reverse_prob`): up to
  `resonance_dsp::MAX_GRAINS` (64) grains. `density_sync` with
  `density_division` spawns one grain per division of the host tempo
  instead, and re-locks on tempo changes.
- **Scheduler** (`scheduler`: Sync, Async, Voice). Sync is regular onsets,
  Async (default) randomises them around the density interval, Voice runs a
  pitch tracker on the input and snaps onsets to pitch marks (PSOLA) while
  the input is voiced, with Async grains underneath as the fallback.
  `align` turns on WSOLA onset alignment (each grain's start is searched
  for the best correlation with the stream).
- **Pitch** (`pitch`, `spread_cents`, `pitch_quantize`: Off, Semi, Scale;
  `root`, `scale` from `resonance-music-theory`): the per-grain transpose
  is quantised at spawn.
- **Feedback** 0–110 % (`feedback`, `fb_route`: Wet→Buf, Out Only (default),
  Pong; `fb_pitch`: cumulative octave-climb vs constant-pitch
  recirculation). The loop has damping (`filter_type` LP/HP, `filter_hz`),
  a tanh soft clip and a DC blocker, so 110 % blooms without running away.
- **Freeze** holds the buffer and resumes with a crossfade.
- **Space:** `diffusion` (four Schroeder allpasses per channel after the
  feedback tap; the knob crossfades dry↔diffused, so 0 is an exact
  identity), `pan_spread` (per-grain pan, decorrelated L/R scheduling),
  `width` (smoothed M/S on the wet sum), `mix`.
- **Quality** (`quality`: Lo-Fi, Norm, HQ). Lo-Fi: linear reads, 8-bit
  µ-law grains and half the grain pool. Norm (default): 4-point Hermite.
  HQ: 6-point Lagrange plus a per-grain anti-alias low-pass on upward
  transposition. Switching tiers mid-stream does not click.
- **Latency:** none reported.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Plugin, block param resolution | `plugins/resonance-granular-delay/src/lib.rs` | `ResonanceGranularDelay` |
| Params | `plugins/resonance-granular-delay/src/params.rs` | 31 params; `DENSITY_MIN_HZ`/`MAX` |
| Choice tables | `src/choice.rs` (`choice_param!`), `src/dsp/modes.rs`, `src/quantize.rs` | `TimeMode`, `Scheduler`, `FbRoute`, `DampingFilter`, `QualityTier`, `PitchQuantize` |
| DSP stages | `src/dsp/{mod,source,grains,voice,time,feedback,diffusion,mix,quant}.rs` | ring buffer, grain engines, feedback loop, diffusion, width/mix |
| Pitch-sync | `src/pitch_sync.rs` | tracker → pitch marks |
| Tempo | `src/sync.rs` | division arithmetic shared with the editor grid |
| Editor state | `src/viz.rs`, `src/dsp/viz_publish.rs` | `GranularViz` |
| Editor | `src/editor/{app,controls,hero,widgets}` | hero buffer view, grouped strip |
| Presets | `src/presets.rs`, `presets/*.json` | 8 factory presets |
| Shared DSP | `resonance-dsp` | `GrainEngine`, `GrainParams`, `SchedulerMode`, `InterpQuality`, `read_hermite_wrapped`, `read_lagrange`, `PitchTracker`, `SwapFader`, `DcBlocker`, `OnePole`, `SimpleRng` |

## Editor

Three bands: a header with live readouts (effective delay, tempo/division,
voice lock, grain count, freeze), the hero buffer view, and a control strip
grouped Time, Grains, Pitch, Feedback, Space, Output. The hero view is
painter-drawn: write head, delay tap, division ticks, pitch ruler, scale
lanes and the live grain cloud. Gestures: drag the tap to change time
(snapping to the tempo grid), drag the cloud to change pitch, scroll to
change density. Each gesture is announced to the host as one edit.

## Factory presets

Init — Per-Grain Cloud; Eighth-Triplet Echo — Tempo-Locked Grains;
D-Minor Shimmer — Scale Quantize + Diffusion; Vocal Doubler — PSOLA + HQ;
Reverse Haze — Ping-Pong + HP Damp; Frozen Drone — Lo-Fi Freeze;
Tape Warble — Repitch + Clean Repeats; Aligned Cloud — WSOLA Onsets.

## Surfaces

- **GUI / Control API / MCP:** the shared plugin surfaces in
  [README.md](README.md#shared-surfaces).
- **Agent skills:** none reference it, and the crate has no
  `tests/skill_keys.rs`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Bit-exact regression (sample bits + per-block digests) | `tests/dsp_regression.rs` → `tests/golden/dsp_regression.u32` | `./scripts/run-tests.py -p resonance-granular-delay` |
| Granulation, time modes, feedback, freeze, stereo, shimmer | `tests/granulate.rs`, `tests/time_modes.rs`, `tests/feedback.rs`, `tests/freeze.rs`, `tests/stereo.rs`, `tests/shimmer.rs` | same |
| Pitch-sync, quantise, alignment, density sync, divisions | `tests/pitch_sync.rs`, `tests/quantize.rs`, `tests/align.rs`, `tests/density_sync.rs`, `tests/sync_divisions.rs` | same |
| Diffusion, quality tiers | `tests/diffusion.rs`, `tests/quality.rs` | same |
| Presets, state round trip, plugin smoke | `tests/presets.rs`, `tests/state.rs`, `tests/plugin.rs` | same |
| Editor groups/labels, knobs, layout, controls, hero geometry, viz | `tests/editor_groups.rs`, `tests/editor_knobs.rs`, `tests/editor_layout.rs`, `tests/editor_controls.rs`, `tests/hero_layout.rs`, `tests/viz.rs`, `tests/viz_snapshot.rs` | same |
| Bench | `benches/granular_dsp.rs` | `cargo bench -p resonance-granular-delay` |

Not covered: no pixel snapshot of the editor; no skill drives it.

## Decisions and deviations

- Every §9 parameter was declared from the first release so the CLAP id
  space stayed stable while the epic landed. All of them are now read by
  the DSP. The comments that still call some of them inert are stale:
  `src/params.rs` header and `src/lib.rs` header mention `TODO(epic-196 …)`
  markers that no longer exist, and `src/editor/hero/interact.rs` says
  `density_sync`'s DSP is "a declared TODO" although `lib.rs` reads it.
- Diffusion sits after the feedback tap: the loop recirculates the
  undiffused wet, so the smear is heard on every repeat but cannot build
  up inside the loop.

## Known gaps

- No agent skill covers it.
