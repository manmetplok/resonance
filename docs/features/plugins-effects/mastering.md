# Mastering (`com.resonance.mastering`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/warmth-width-depth.md` §6.3 (slice W9: EQ M/S, per-band width, clipper, sat modes, inflator; W10 assistant; W12 de-harsh) and `docs/history/specs/deharsh-resonance-suppressor.md`. Audit findings M1–M5 and P8 in `docs/history/reviews/plugin-audit.md` were fixed since; M6 is still open.

A complete mastering chain in one plugin for the master bus. Ten stages run
in a fixed order, and every one is off by default, so an unconfigured
instance measures the same as an empty master. Next to the chain sit a
loudness/peak/spectrum metering view and an assistant that suggests settings
from a genre target band or a reference track. ARCHITECTURE.md uses this
crate as the template for decomposing a large component.

## Behaviour

- **Chain** (`src/chain.rs`), in order: input trim → corrective EQ →
  de-harsh → glue compressor → saturator → tonal EQ → multiband → imager
  → clipper → limiter → dither → metering tap.
  - *Input trim* (`input_trim_db`): the level every stage sees.
  - *Corrective / tonal EQ* (`corr_b{n}_*`, `tone_b{n}_*`, n = 0–3; type
    Bell, Low Shelf, High Shelf, High-pass, Low-pass; `_ms` Stereo / Mid /
    Side): linear-phase. The biquad chain's magnitude is sampled onto an FFT
    grid and becomes a 4097-tap zero-phase FIR (`FIR_LENGTH`) in an
    overlap-save convolver. A band change is designed on a background
    worker (or inline if the worker is late) and crossfaded over one hop.
    A Mid or Side band engages a second, cross convolver pair.
  - *De-harsh* (`dh_*`): the STFT resonance suppressor, see
    [deharsh.md](deharsh.md).
  - *Glue compressor* (`glue_*`: threshold, ratio, attack, release, knee,
    makeup, mix). The detector tracks the louder channel.
  - *Saturator* (`sat_mode`: Blend, Tube, Tape, Transformer, Console, Warm,
    Inflator; `sat_drive`, `sat_mix`; `sat_character` and `sat_shaper` in
    Blend only; `sat_curve` in Inflator only; `sat_auto_gain`, default off).
    The non-Blend modes run in a 4x IIR oversampler.
  - *Multiband* (`mb_on`, crossovers `mb_xo1..3`, per band n = 0–3:
    `mb_b{n}_on`, `_thresh`, `_ratio`, `_attack`, `_release`, `_knee`,
    `_mix`, `_gain`): four bands on linear-phase crossovers, each a full
    `GlueCompressor`. Band gain applies even with the band's compressor off.
    Per-band gain reduction is published and drawn.
  - *Imager* (`img_on`, `img_width` 0–2, `img_side_hpf_on/_freq` as the
    mono-maker, `img_b{n}_width` per multiband band, applied before the
    bands are summed).
  - *Clipper* (`clip_on`, `clip_drive`, `clip_shape` hard↔soft): ADAA curve
    in an 8x IIR oversampler. It never adds loudness on its own.
  - *Limiter* (`lim_on`, `lim_ceiling` dBTP, `lim_release`, `lim_gain`
    0–18 dB pushed in after the clipper): brick-wall true-peak limiter with
    a fixed 5 ms lookahead and 4x oversampled BS.1770-4 true-peak detection.
  - *Dither* (`dith_on`, `dith_bits`, `dith_ns`): TPDF dither, optional
    first-order noise shaping that requantises to the target depth.
- **Whole-plugin bypass** (`bypass`, a CLAP bypass param) crossfades over
  10 ms to a latency-matched dry signal, with the stages kept warm.
- **Latency** is constant per activation, whatever is switched on: the two
  linear-phase EQs, the de-harsh frame (2048 samples at ≤ 48 kHz), the
  multiband crossover and the limiter lookahead (`Chain::latency`). The
  clipper and the non-Blend saturator add about 6.9 and 5.5 samples of
  frequency-dependent group delay that cannot be reported, so with both on
  the chain runs about 12 samples behind its reported latency. Where such a
  stage is crossfaded (bypass, the two stages' own enables) the 10 ms fade
  combs briefly. The module comment explains why that is the lesser evil.
- **Param layout:** the first 102 params are in signal order. W9 and later
  additions are appended, so older automation lanes keep their index.
  `target_lufs` is the loudness reference line drawn by the LUFS meter,
  the LUFS history and the header. The assistant writes it on "Apply
  suggestions"; otherwise the user sets it from the header.

## Assistant

The audio thread copies the post-chain signal into a 10-second stereo ring.
On demand the panel snapshots it and analyses it, then compares it with a
genre target band (Rock, Indie, Acoustic, Jazz, Pop) or a loaded reference
file. It shows suggestions per stage with their rationale, and applies them
in one click. The analysis and decisions are the shared
[mastering-assist](mastering-assist.md) engine. The plugin owns the capture,
the reference decoding, the panel state and the key → param lookup
(`decide::param_by_key`). The chosen target (genre or reference, and which
one) is saved with the plugin state. The assistant processes no audio.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Plugin | `plugins/resonance-mastering/src/lib.rs` | `ResonanceMastering`, `latency_samples` |
| Chain | `plugins/resonance-mastering/src/chain.rs` | `Chain`, `latency`, bypass crossfade, `deharsh_max_cut_db` |
| Stages | `plugins/resonance-mastering/src/stages/` | `linear_phase_eq/{mod,convolver,design,worker}`, `deharsh`, `glue_compressor`, `saturator`, `sat_modes`, `multiband/`, `imager`, `clipper`, `limiter`, `dither` |
| Params | `plugins/resonance-mastering/src/params/` | `MasteringParams` (`mod.rs`), one file per stage |
| Metering tap | `plugins/resonance-mastering/src/dsp.rs`, `src/viz.rs` | LUFS M/S/I, true peak, correlation, spectrum |
| Assistant | `plugins/resonance-mastering/src/assistant/{capture,decide,reference,state}.rs` | `Assistant`, `AssistantStateSaver`, `STATE_KEY` |
| Editor | `plugins/resonance-mastering/src/editor/` | `header.rs` (bypass, ref line), `controls/` (one panel per `StageTab`), `spectrum.rs`, `lufs_meter.rs`, `lufs_history.rs`, `tp_meter.rs`, `tp_history.rs`, `correlation.rs`, `readouts.rs` |
| Shared DSP | `resonance-dsp`, `resonance-metering`, `resonance-mastering-assist` | `FftConvolver`, `Biquad`/`BiquadCoeffs`/`BandType`, `Curve`, `Adaa`, `DcBlocker`, `soft_knee_gain_reduction_db`, `Ballistics`, `deharsh::ResonanceSuppressor`, `hann_window`, `SimpleRng` |

## Editor

A header with whole-plugin bypass, the loudness reference line and
readouts (integrated LUFS, glue GR, limiter GR). A tab
bar with one tab per stage, Assistant first (Assistant, Corrective EQ,
De-harsh, Glue, Saturator, Tonal EQ, Multiband, Imager, Clipper, Limiter,
Dither). The selected stage's panel. Then the meters: 1/6-octave spectrum,
LUFS M/S/I strip, stereo true-peak bars, correlation, crest / PLR / PSR /
LRA readouts, and rolling LUFS-M and true-peak traces. The preset bar holds
user presets only.

## Factory presets

None. The plugin declares no `FACTORY_PRESETS`.

## Surfaces

- **GUI / Control API / MCP:** the shared plugin surfaces in
  [README.md](README.md#shared-surfaces), on the master:
  `master.add_effect` → `mcp__resonance__master_add_effect` with
  `com.resonance.mastering`, then `mcp__resonance__master_set_plugin_param`
  by key, `mcp__resonance__master_plugin_params` to read ranges.
- **Assistant over the wire:** `master.assist` →
  `mcp__resonance__master_assist` runs the same engine on an offline render
  and returns suggestions without applying them. The app side is in the
  mixer area, see [mixer/mastering-assist.md](../mixer/mastering-assist.md) and
  [mastering-assist.md](mastering-assist.md).
- **Agent skills:** `mastering/SKILL.md` (the full stage → key map, "enable
  one stage at a time, measure after each"; plus `reverb` keys for a glue
  insert before the chain). `mixing/references/reading-meters.md` explains
  reading `master_assist` and applying its writes.

## How it's tested

| What | Where | Run |
|---|---|---|
| Bit-exact output | `tests/dsp_golden.rs` → `tests/golden/dsp_golden.f32`; W9 stages `tests/w9_golden.rs` → `golden/w9_golden.f32` | `./scripts/run-tests.py -p resonance-mastering` |
| Pre-W9 state renders bit-identical | `tests/legacy_state.rs` → `golden/legacy_state.f32` | never re-bless for a "transparent" change |
| Per stage | `tests/stages_{clipper,deharsh,dither,glue_compressor,imager,limiter,saturator,multiband,multiband_lowpass,multiband_band_controls,linear_phase_eq,linear_phase_eq_convolver,linear_phase_eq_design,fir_design_worker}.rs`, `tests/sat_modes.rs`, `tests/saturator_probe.rs`, `tests/eq_mid_side.rs`, `tests/imager_band_width.rs`, `tests/multiband_band_gr.rs` | same |
| RT safety, convolver hop stagger | `tests/linear_phase_rt_alloc.rs`, `tests/chain_convolver_stagger.rs` | same |
| Whole chain | `tests/integration.rs` | same |
| Assistant: analyse, decide, targets, capture, state, emitted keys resolve | `tests/assistant_{analyze,decide,targets,capture,state,lockstep}.rs` | same |
| Labels and units, editor header, multiband panel | `tests/params_labels.rs`, `tests/editor_header.rs`, `tests/editor_multiband_panel.rs` | same |
| Skill keys | `tests/skill_keys.rs` | same |
| Bench | `benches/mastering_dsp.rs`, `benches/dsp.rs` | `cargo bench -p resonance-mastering` |

Not covered: there is no `state.rs` round trip of every param (the
assistant state and legacy load are tested), and no pixel snapshot of the
editor.

## Decisions and deviations

- Constant reported latency, including de-harsh when off, so no stage
  toggle ever shifts the timeline (deharsh spec §5.1, F2 policy).
- IIR oversampling only (warmth spec D3). The resulting group delay is
  documented rather than compensated.
- The saturator's character modes share `resonance-dsp` primitives with
  [Color](color.md), not code.
- Stale module comment: `src/editor/mod.rs` still describes the
  "Phase 2" analyzer-only editor ("No processing controls yet — those
  arrive in Phase 3").

## Known gaps

- Limiter lookahead is a fixed 5 ms (`LOOKAHEAD_MS`); the limiter panel
  prints it as static text (audit M6).
- The de-harsh stage's live cut is not shown anywhere:
  [deharsh-cut-readout](../../intents/plugins-effects/deharsh-cut-readout.md).
