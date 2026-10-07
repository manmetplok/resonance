# De-harsh resonance suppressor

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/deharsh-resonance-suppressor.md` (the W12 design note), `docs/history/specs/warmth-width-depth.md` §6.3 and decision D4.

A soothe-style resonance suppressor: many narrow cuts that come and go, each
on a peak that stands out of the smoothed spectrum, so harsh resonances in
the presence range are tamed while a broadband mix passes nearly untouched.
The core lives in `resonance-dsp` so other plugins can reuse it. Today it
ships only as the **De-harsh** stage of the [mastering](mastering.md) chain,
between the corrective EQ and the glue compressor. For a track or bus,
the skills use an EQ dynamic bell at 0 dB instead (see [eq.md](eq.md)).

## Behaviour

- **STFT gain mask.** A periodic Hann window of 2048 samples at ≤ 48 kHz
  (doubled per octave of sample rate above that, up to 8 × 48 kHz), hop
  frame/8 (87.5 % overlap), Hann synthesis, weighted overlap-add. Both
  channels ride one complex FFT.
- **Detection spectrum D.** Per-bin power averaged over a near-triangular
  kernel as wide as the cut (`f / Q`, at least 3 bins), integrated over
  10 ms (`DETECTOR_TAU_MS`).
- **Reference R.** A peak-excluded, log-frequency-uniform one-octave moving
  average of D in dB: average, clip D to it, average again. A steady
  broadband signal sits on its own reference and is not cut.
- **Cut.** The excess `E = D − R` above `selectivity`, through a fixed
  4 dB soft knee (`KNEE_DB`) with infinite ratio, capped at `depth`,
  tapered at the band edges (`low`…`high`), smoothed per bin with
  attack/release at the hop rate. A steady peak E dB above its reference
  ends at `max(T, E − depth)` dB above it. Nothing below a −100 dB
  reference floor is cut.
- **Modes:** Stereo, Mid, Side, Mid+Side. `mix` is a latency-aligned
  wet/dry. `delta` outputs only what is removed (for listening, never to
  leave on).
- **Latency is exactly one frame in every state**, including off, depth 0,
  mix 0 and delta. Off is the input delayed by that frame, bit-exact, and
  the STFT keeps running underneath, so switching on crossfades (10 ms)
  onto a warm path.
- **RT safety.** Everything is sized in `ResonanceSuppressor::new`.
  Processing never allocates (rustfft with scratch). A non-finite input
  sample no longer latches a full-depth cut forever.
- **Mastering stage params** (`plugins/resonance-mastering/src/params/deharsh.rs`,
  1:1 onto `SuppressorConfig`, same defaults): `dh_on` (off),
  `dh_depth` 0–24 dB (6), `dh_selectivity` 0–18 dB (5), `dh_sharpness`
  Q 3–24 (24), `dh_attack` 5–200 ms (10), `dh_release` 20–1000 ms (100),
  `dh_low` / `dh_high` 200 Hz–20 kHz (1 kHz / 8 kHz), `dh_mode`, `dh_mix`,
  `dh_delta`. The defaults are pinned to the W12 exit criterion: a Q 10
  resonance 15 dB up is cut 7.8 dB while broadband noise moves less than
  0.2 dB.
- **FFT stagger.** In the mastering chain the suppressor's frames run at
  3/4 of each hop (`set_phase_offset`), so they fall on different
  callbacks from most of the linear-phase EQ convolvers' FFTs.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Core | `resonance-dsp/src/deharsh.rs` | `ResonanceSuppressor` (`new`, `process_*`, `latency`, `max_cut_db`, `cut_db`, `set_phase_offset`, `reset`), `SuppressorConfig` (+ `sanitized`), `SuppressorMode` (`NAMES`), `StftGeometry`, the `*_RANGE` constants |
| Mastering stage | `plugins/resonance-mastering/src/stages/deharsh.rs` | `DeharshStage` |
| Mastering params | `plugins/resonance-mastering/src/params/deharsh.rs` | `dh_*`, `MODE_LABELS` |
| Mastering panel | `plugins/resonance-mastering/src/editor/controls/deharsh.rs` | the De-harsh tab |
| Chain position, latency | `plugins/resonance-mastering/src/chain.rs` | `Chain::latency`, `deharsh_max_cut_db` |

## Surfaces

- **GUI:** the De-harsh tab of the mastering plugin's editor.
- **Control API / MCP:** `mcp__resonance__master_set_plugin_param` with the
  `dh_*` keys on `com.resonance.mastering`.
- **Agent skills:** `mastering/SKILL.md` (the `dh_*` row of the stage map,
  and "a presence peak → de-harsh at the defaults"). `mixing/SKILL.md`
  ("De-harsh first") does the move on busses and tracks with a dynamic EQ
  bell, not with this suppressor.

## How it's tested

| What | Where | Run |
|---|---|---|
| Design-note tests T1–T12 (cut depth on a resonance, broadband left alone, latency, transparency off, no allocation via a counting global allocator) | `resonance-dsp/tests/deharsh.rs` (own binary) | `./scripts/run-tests.py -p resonance-dsp` |
| NaN recovery | `resonance-dsp/tests/deharsh_nonfinite.rs` | same |
| In the chain: params, constant latency, off is bit-transparent, the cut through the whole chain, the panel | `plugins/resonance-mastering/tests/stages_deharsh.rs` | `./scripts/run-tests.py -p resonance-mastering` |
| Pinned output | `plugins/resonance-mastering/tests/dsp_golden.rs`, `w9_golden.rs`; old states via `legacy_state.rs` | same |
| Skill key names | `plugins/resonance-mastering/tests/skill_keys.rs` | same |

Not covered: the defaults are measured, not listened to (design note §7.3
asked for a listening pass before the skills quote them). Musical noise on
dense material (§7.1) has no test.

## Decisions and deviations

- A full STFT resonance suppressor, not a dynamic-EQ stand-in (D4). The EQ's
  dynamic bands stay a separate, simpler feature.
- Constant one-frame latency, even when off (design note §5.1, open
  question 4). The alternative, a latency change plus host restart on
  every `dh_on` toggle, would save about 43 ms on masters that never use
  the stage, at the cost of a dropout per toggle.
- Defaults Q 24 / selectivity 5 instead of the draft's Q 8 / T 6, which
  cut the test resonance by only 3.5 dB.
- The core's and the mastering crate's comments still point to
  `docs/history/specs/deharsh-resonance-suppressor.md`. That file is now
  `docs/history/specs/deharsh-resonance-suppressor.md`.

## Known gaps

- The live cut (`max_cut_db`, `cut_db`) is computed but shown nowhere, in
  the editor or on the wire:
  [deharsh-cut-readout](../../intents/plugins-effects/deharsh-cut-readout.md).
- No track or bus insert of the suppressor exists:
  [deharsh-track-insert](../../intents/plugins-effects/deharsh-track-insert.md).
