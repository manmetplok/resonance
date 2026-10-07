# Color (`com.resonance.color`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/warmth-width-depth.md` §6.1 (slices W5 primitives, W6, W6b Tape HQ; decisions D1–D3).

The character plugin for tracks and busses: one saturation stage in five
voicings (Tube, Tape, Transformer, Console, Warm), with a drive tilt, an
output tilt, parallel mix and auto-gain. Auto-gain is on by default, so a
before/after judgement is made at matched loudness. Tape mode has a second,
physically modelled quality (Jiles-Atherton hysteresis). The editor shows
the mode's transfer curve and its measured harmonic signature, and the
factory presets are voiced against that same measurement.

## Behaviour

- **Mode** (`mode`: Tube, Tape, Transformer, Console, Warm).
  - *Tube*: biased curve, H2-dominant.
  - *Tape*: soft curve, then a head bump and level-dependent HF loss after
    the downsampler; `speed` (7.5 / 15 / 30 ips) sets the head-bump and
    HF-loss corners; `flutter` adds transport wow.
  - *Transformer*: an LF-weighted drive stage (drives the lows harder,
    sub-sonic high-pass, HF resonance).
  - *Console*: odd-only, very clean, for glue at low drive.
  - *Warm*: a strictly even curve (even harmonics only).
- **Shaping:** `drive` (0–1), `bias` (shapes the modes that use it),
  `response` (±12 dB drive tilt), `tone` (±6 dB output tilt), `mix`
  (parallel), `auto_gain` (default on), `output` (−24…+12 dB).
- **Oversampling** (`oversample`: Off, 2x (default), 4x) uses the
  latency-free IIR half-band `Oversampler`. The curve runs with
  first-order ADAA inside it. The dry path runs through a matching
  up/down pair, so `mix` never comb-filters.
- **Tape HQ** (`tape_quality`: Standard, HQ; `tape_solver`: RK2, RK4
  (default), NR). HQ swaps the Tape curve for a Jiles-Atherton hysteresis
  stage per channel, forced to at least 2x oversampling. Head bump, HF
  loss, flutter, tone and auto-gain are shared with Standard. Entering HQ
  starts the loop demagnetised.
- **Auto-gain** holds the output within ±0.5 LU of the input (BS.1770,
  gated) on pink noise and a drum loop, in every mode and across the drive
  range. Its followers have a 0.8 s time constant.
- **Flutter** glides the whole output delay (about 1.1 ms centre) in and
  out over `FLUTTER_ENGAGE_MS` instead of switching. At 0 it is bypassed
  outright.
- **Exact paths:** `mix` 0 returns the dry path exactly (the input itself
  at Off). Console at drive 0 with neutral tilts is the identity.
- **Latency:** reported 0 at every oversample setting and in both tape
  qualities, so changing the factor never needs a host restart. Flutter's
  ~1.1 ms is not reported: it exists only in Tape with flutter above 0,
  and it is part of the effect.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Plugin | `plugins/resonance-color/src/lib.rs` | `ResonanceColor` |
| Params | `plugins/resonance-color/src/params.rs` | `ColorParams`, `Mode::LABELS`, `SPEED_LABELS`, `TapeQuality::LABELS` |
| DSP | `plugins/resonance-color/src/dsp/mod.rs` | `ColorDsp`, `Settings` (incl. `stage_factor`) |
| Voicings | `plugins/resonance-color/src/dsp/voicing.rs` | `transfer_for`, `tape_hq` (Jiles-Atherton params) |
| Harmonic probe | `plugins/resonance-color/src/probe.rs` | 1 kHz sine at −18 dBFS through the current settings, single-bin DFT per harmonic |
| Editor state | `plugins/resonance-color/src/viz.rs` | `ColorViz`: input/output peaks, auto-gain dB |
| Editor | `plugins/resonance-color/src/editor/{app,curve,harmonics,controls}.rs` | transfer curve with the live input peak, harmonic bars, control strip |
| Presets | `plugins/resonance-color/src/presets.rs`, `presets/*.json` | 5 factory presets |
| Shared DSP (W5 primitives) | `resonance-dsp` | `Curve`, `Adaa1`, `Oversampler`/`OversampleFactor`, `LfWeightedDrive`, `Hysteresis`/`HysteresisSolver`/`JaParams`, `EmphasisPair`, `HeadBump`, `HfLoss`, `Flutter`, `tape::{head_bump_hz, hf_loss_corner_hz}`, `DcBlocker`; `resonance_metering::k_weighting::KWeightingFilter` for auto-gain |

## Editor

A header, a centre row with the transfer curve (the mode's static curve,
with the live input peak marked on it) and the harmonic bars, and a control
strip. The bars come from `probe.rs`, the same probe that
`tests/harmonics.rs` pins and the presets were voiced against. The probe
forces auto-gain off and flutter to 0. It runs on a background worker
(`editor/harmonics.rs`), and both plots recompute only when a setting they
depend on changes.

## Factory presets

Bus — Warm Glue, Bass — Iron, Vocal — Tube Air, Drums — Tape 15,
Master — Subtle Tape. The skills cite these names.

## Surfaces

- **GUI / Control API / MCP:** the shared plugin surfaces in
  [README.md](README.md#shared-surfaces). An agent sets drive against a THD
  target with `meter.probe` → `mcp__resonance__meter_probe` (the harmonic
  signature of an insert chain), and judges a move with `meter_snapshot` +
  `meter_compare`.
- **Agent skills:** `mixing/references/character.md` (two keys blocks:
  which preset and voicing per bus or track, and the knobs the warmth
  steps turn: `drive`, `mode`, `bias`, `oversample`). `mixing/SKILL.md`
  step 3 puts it on busses, not the master.

## How it's tested

| What | Where | Run |
|---|---|---|
| Bit-exact output | `tests/dsp_golden.rs` → `tests/golden/dsp_golden.f32` | `./scripts/run-tests.py -p resonance-color` |
| Harmonic signature per mode (structure + pinned H2..H5, THD) | `tests/harmonics.rs` | same |
| Auto-gain within ±0.5 LU | `tests/auto_gain.rs` | same |
| Transparency (`mix` 0, Console identity) | `tests/transparency.rs` | same |
| Tape HQ (loop shape, stability, HQ vs Standard) | `tests/tape_hq.rs` → `golden/tape_hq.f32` | same |
| Pre-W6b state renders unchanged | `tests/legacy_state.rs` → `golden/legacy_state.f32` | never re-bless for a "transparent" change |
| Zero latency at every factor through the real CLAP ABI | `tests/host.rs` | same |
| NaN hardening, state round trip, presets | `tests/nan_hardening.rs`, `tests/state.rs`, `tests/presets.rs` | same |
| Curve, bars and meter feed | `tests/viz.rs` | same |
| Editor binding, plugin smoke, skill keys | `tests/editor_param_binding.rs`, `tests/plugin.rs`, `tests/skill_keys.rs` | same |
| Tape HQ CPU sanity (`#[ignore]`; HQ at most 10x Standard at 2x) | `tests/tape_hq.rs` | `cargo test --release -p resonance-color --test tape_hq -- --ignored --nocapture` |

Not covered: no pixel snapshot of the editor. The voicings are measured, not
listened to. The warmth build closed with a by-hand look at the editors and
a live agent session still open (see the intents below).

## Decisions and deviations

- A separate plugin rather than growing the EQ or the mastering
  saturator (D1). The mastering `sat_*` modes share the `resonance-dsp`
  primitives, not code paths.
- Standard and HQ tape both exist (D2). Oversampling is IIR-only and
  latency-free (D3).
- The spec's PurestWarm one-polarity curve was not even-only, so `Warm`
  became a strictly even curve instead.
- The dry path is deliberately not run through a matching ADAA average.
  That would soften the top octave at every mix, including 0.

## Known gaps

- Editor visuals never checked in a live session:
  [plugin-editor-visual-checks](../../intents/plugins-effects/plugin-editor-visual-checks.md).
- No live agent warmth session on a real project:
  [effects-live-agent-session](../../intents/plugins-effects/effects-live-agent-session.md).
