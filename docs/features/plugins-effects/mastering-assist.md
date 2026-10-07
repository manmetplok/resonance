# Mastering assistant engine (`resonance-mastering-assist`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/warmth-width-depth.md` §7.4–7.5 (slice W10; decision D6 on genre bands).

The library that decides what the mastering chain should do to a mix. Given
two channels of audio, it measures loudness, true peak, crest, correlation
and a 1/6-octave long-term average spectrum. It compares that with a target
and returns suggestions stage by stage, each with a rationale and the exact
mastering-plugin param writes it amounts to. The target is a built-in genre
band or a reference track. It has two callers: the mastering plugin's
Assistant panel (live capture, applies the writes) and the app's
`master.assist` (offline render, returns the writes without applying them).
Because both depend on this crate, the app links no plugin.

## Behaviour

- **Analysis** (`analyze::run(sample_rate, left, right)` →
  `AnalysisResult`): BS.1770 integrated loudness, true peak, crest factor
  and L/R correlation over the range, and a Welch LTAS on the 1/6-octave
  grid (`NUM_SPECTRUM_BINS`, silent bins floored at `FLOOR_DB`). Every
  figure comes from `resonance-metering`, the same functions the engine's
  offline measurement uses, so the panel and the wire agree on the same
  audio.
- **Targets** (`decide::Target`):
  - *Genre* (`targets::Genre`: Rock, Indie, Acoustic, Jazz, Pop; ids
    `rock` … `pop`). A target is a band: a min and max level per ISO
    1/3-octave band, 20 Hz–20 kHz (`NUM_TARGET_BANDS` = 31). The slope comes
    from published averages (Pestana et al., about 4.5–5 dB/oct between
    100 Hz and 4 kHz), with small reasoned genre offsets at the extremes
    and tolerances widest at the extremes. Integrated targets: Rock −11,
    Indie −13, Acoustic −16, Jazz −17, Pop −10 LUFS.
  - *Reference* (`reference::ReferenceTrack`): the reference's LTAS becomes
    the shape, with only ±`REFERENCE_TOLERANCE_DB` (1 dB) slack, and its
    integrated LUFS becomes the loudness target. Getting the audio is the
    caller's job.
- **Decision** (`decide::build(&analysis, &target)` → `Suggestions`): align
  the analysed spectrum's 400 Hz–2.5 kHz midrange (`MID_BAND_HZ`) to the
  band's midline (only shape matters, never level), measure what falls outside the band in the low (20–100 Hz) and
  high (5–20 kHz) regions, and suggest tonal low/high shelves for it. Then
  suggest glue compressor settings, imager width and side HPF, and limiter
  ceiling, release and input gain. The loudness gap is split: the input
  trim stops 3 dB short of the target and the limiter's `lim_gain` pushes
  the rest, so loudness is made after the clipper. Every rationale line is
  tagged with its stage: `input_trim`, `tonal_low_shelf`,
  `tonal_high_shelf`, `glue`, `imager`, `limiter`, `target_lufs`, plus
  `diagnostic` notes that carry no writes (`STAGE_*` constants). `deviations` reports the per-band comparison.
- **Output as param writes.** `Suggestions::stages()` gives
  `StageSuggestion`s, each a list of `ParamChange { key, value }` by
  mastering-plugin key. `EMITTED_PARAM_KEYS` lists every key that can be
  emitted (input trim, tonal bands 0 and 3, glue, imager, limiter,
  `target_lufs`). `Suggestions::apply_to(&impl ParamSink)` writes them to
  any sink. The plugin implements `ParamSink` over its own params.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Crate root | `resonance-mastering-assist/src/lib.rs` | re-exports `AnalysisResult`, `ParamSink`, `Suggestions`, `Target`, `ReferenceTrack`, `Genre` |
| Analysis | `resonance-mastering-assist/src/analyze.rs` | `run`, `AnalysisResult`, `NUM_SPECTRUM_BINS` |
| Genre bands | `resonance-mastering-assist/src/targets.rs` | `Genre`, `genre_bands`, `target_lufs`, `target_band_center_hz`, `NUM_TARGET_BANDS` |
| Decisions | `resonance-mastering-assist/src/decide.rs` | `build`, `Target`, `Suggestions`, `StageSuggestion`, `ParamChange`, `ParamSink`, `EMITTED_PARAM_KEYS`, `REFERENCE_TOLERANCE_DB` |
| Reference | `resonance-mastering-assist/src/reference.rs` | `ReferenceTrack::from_samples` |
| Plugin caller | `plugins/resonance-mastering/src/assistant/` | capture ring, `decide::param_by_key`, panel state (see [mastering.md](mastering.md)) |
| App caller | `resonance-app/src/update/control/assist.rs` | `master.assist` job handler — see [mixer/mastering-assist.md](../mixer/mastering-assist.md) |

Layering (ARCHITECTURE.md): depends on `resonance-dsp` and
`resonance-metering` only. `tools/arch-invariants` enforces that the app
reaches it as a library, never through the plugin crate.

## Surfaces

- **GUI:** the Assistant tab of the mastering plugin's editor.
- **Control API / MCP:** `master.assist` → `mcp__resonance__master_assist`
  (`mode` `genre` with `genre`, or `reference` with a `pool_asset_id`;
  optional `range`). It runs as a job on an offline render of the master.
  In reference mode the pooled asset is first measured as decoded audio.
  The result is the suggestions; nothing is applied. The agent applies what
  it agrees with via `mcp__resonance__master_set_plugin_param`. Wire types
  are in `resonance-control/src/methods/master.rs`; the app-side handler
  (job shape, reference-mode sequencing, result fields) is documented in
  [mixer/mastering-assist.md](../mixer/mastering-assist.md).
- **Agent skills:** `mastering/SKILL.md` and
  `mixing/references/reading-meters.md` (how to read `master_assist` and
  apply its writes).

## How it's tested

| What | Where | Run |
|---|---|---|
| Emitted keys = `EMITTED_PARAM_KEYS` | `resonance-mastering-assist/tests/emitted_keys.rs` | `./scripts/run-tests.py -p resonance-mastering-assist` |
| Loudness split between trim and `lim_gain` | `resonance-mastering-assist/tests/loudness.rs` | same |
| Every emitted key resolves to a real mastering param | `plugins/resonance-mastering/tests/assistant_lockstep.rs` | `./scripts/run-tests.py -p resonance-mastering` |
| Analysis, decisions, targets, capture, panel state | `plugins/resonance-mastering/tests/assistant_{analyze,decide,targets,capture,state}.rs` | same |
| `master.assist` over the wire: engine commands, payload = the panel's writes; the engine's measurement of a buffer equals `analyze::run` of it (`the_engine_measurement_is_the_panels_analysis`) | `resonance-app/tests/control/control_master_assist.rs` (group `control`) | `./scripts/run-tests.py -p resonance-app` |

Not covered: the parity test uses a synthetic buffer, not the plugin's live
capture ring against an offline render of the same real mix. The genre bands are reasoned from published averages and have
not been checked against real masters in this repo; there is no golden of a
full suggestion set for a fixed real mix.

## Decisions and deviations

- Genre targets are built-in bands from published averages. Reference
  tracks compare against but never generate targets (D6).
- The engine lives in its own crate so the app does not depend on a plugin
  crate (ARCHITECTURE.md layering rule).

## Known gaps

- Suggestions touch only tonal bands 0 and 3, glue, imager, limiter, trim
  and `target_lufs`. The de-harsh, saturator, multiband and clipper stages
  are never suggested.
