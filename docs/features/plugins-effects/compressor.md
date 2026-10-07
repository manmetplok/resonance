# Compressor (`com.resonance.compressor`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/warmth-width-depth.md` §6.4 (`release_mode` Auto); `docs/history/reviews/plugin-audit.md` C1/C2 (sidechain key).

A stereo feed-forward compressor for tracks and busses. It has a soft knee,
a detector that blends peak and RMS, a sidechain high-pass on the detector,
an external sidechain key, parallel mix, manual and auto makeup, and a
program-dependent "Auto" release for bus glue. The editor shows the transfer
curve, a gain-reduction history and In / Key / GR / Out meters.

## Behaviour

- **Signal path** (`src/dsp.rs`): detector → static soft-knee gain
  computer → attack/release ballistics → gain → makeup → dry/wet mix. All
  of it after the detector runs in dB.
- **Detector.** The source is the key when one is connected, else the
  input. Each detector channel can run through its own sidechain high-pass
  (`sc_hpf_on`, `sc_hpf_freq`). The louder of the two filtered channels
  feeds a fast peak envelope and a 30 ms RMS envelope. `detector_mix`
  crossfades the two in dB. Tracking the louder channel, not the mono sum,
  keeps hard-panned and anti-phase material from reading low.
- **Gain computer:** `threshold`, `ratio`, `knee` (soft knee via
  `resonance_dsp::soft_knee_gain_reduction_db`).
- **Release modes** (`release_mode`: Manual, Auto). Manual is a single
  envelope with the user's `release`. Auto runs two GR envelopes and the
  deeper wins: a fast one (user attack, 50 ms release) and a slow one that
  charges over 300 ms and releases over 1500 ms. Short transients recover
  fast and sustained compression recovers slowly. `release` is ignored in
  Auto.
- **Release time is honest.** After a loud passage, GR falls to 1/e in one
  release time constant. The peak detector's own decay is short and fixed,
  so release is not applied twice.
- **Output:** `makeup` (dB), `auto_makeup`, `mix` (parallel). Makeup and
  mix are smoothed, so automation steps do not zipper.
- **Sidechain key.** `SIDECHAIN_INPUT = Some(2)`. The key only replaces
  the detector source and never reaches the output. Whether a key is
  connected and the level it reaches at the detector are published to the
  editor every block.
- **Latency:** none. A non-finite input sample cannot wedge the envelopes.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Plugin + key port | `plugins/resonance-compressor/src/lib.rs` | `ResonanceCompressor`, `process_with_key` |
| Params | `plugins/resonance-compressor/src/params.rs` | `threshold`, `ratio`, `attack`, `release`, `knee`, `makeup`, `mix`, `detector_mix`, `sc_hpf_freq`, `sc_hpf_on`, `auto_makeup`, `release_mode`; `RELEASE_MODE_LABELS` |
| DSP | `plugins/resonance-compressor/src/dsp.rs` | detector, gain computer, `AUTO_*_MS` constants |
| Editor state | `plugins/resonance-compressor/src/viz.rs` | `CompressorViz` (GR history ring, `key_connected`, key level, `KEY_METER_LABEL`) |
| Editor | `plugins/resonance-compressor/src/editor/{app,curve,history,meters,control_strip}.rs` | transfer curve, GR trace, meters, knob strip |
| Presets | `plugins/resonance-compressor/src/presets.rs`, `presets/*.json` | 11 factory presets |
| Shared DSP | `resonance-dsp` | `soft_knee_gain_reduction_db`, `Ballistics`, `Biquad`, `db_to_linear`, `linear_to_db` |

## Editor

The transfer curve's dB window comes from the `threshold` param's declared
range, so every reachable threshold is on the plot. The meters show input,
key (only while a key is connected, labelled "KEY/DET", showing the detector
level), GR (hanging from the top, 0–24 dB) and output. The meter that feeds
the detector carries the threshold marker. The header names the detector
source ("DETECTOR: SIDECHAIN KEY" when keyed). Knobs are
`editor_widgets::float_knob`, which reads range, skew and default from the
param.

## Factory presets

Kick — Punch, Snare — Slam, Bass — Glue, Vocal — Lead, Guitar — Control,
Drum Bus, Mix Bus, Master — Glue, Parallel Smash, Transparent,
Bus — Auto Glue.

## Surfaces

- **GUI / Control API / MCP:** the shared plugin surfaces in
  [README.md](README.md#shared-surfaces). Keying it:
  `track.set_sidechain` / `bus.set_sidechain` / `master.set_sidechain` →
  `mcp__resonance__track_set_sidechain` etc.
- **Agent skills:** `mixing/references/character.md` (bus glue: start from
  `Bus — Auto Glue`, or ratio/attack/`release_mode` `Auto`/threshold/
  `auto_makeup`/`mix` by hand). The `mixing` SKILL.md also names it for
  dynamics generally.

## How it's tested

| What | Where | Run |
|---|---|---|
| Bit-exact output | `tests/dsp_golden.rs` → `tests/golden/dsp_golden.f32` | `./scripts/run-tests.py -p resonance-compressor` |
| Auto release | `tests/auto_release.rs` → `golden/auto_release.f32` | same |
| Pre-`release_mode` state renders unchanged | `tests/legacy_state.rs` → `golden/legacy_state.f32` | never re-bless for a "transparent" change |
| Release time constant | `tests/release_time.rs` | same |
| Stereo detector | `tests/stereo_detector.rs` | same |
| Sidechain key and key meter | `tests/sidechain_key.rs`, `tests/key_meter.rs` | same |
| Smoothing, NaN hardening | `tests/smoothing.rs`, `tests/nan_hardening.rs` | same |
| Editor: knob ↔ param binding, curve axis, GR ring | `tests/editor_param_binding.rs`, `tests/curve_axis.rs`, `tests/viz.rs` | same |
| Plugin smoke, skill keys | `tests/plugin.rs`, `tests/skill_keys.rs` | same |
| Bench | `benches/compressor_dsp.rs` | `cargo bench -p resonance-compressor` |

Not covered: there is no `state.rs` save/load round trip (gate, delay,
reverb, stereo and color have one) and no pixel snapshot of the editor.
`benches/dsp.rs` is an empty file.

## Decisions and deviations

- "Sidechain HPF" (an internal filter on the detector) and "sidechain key"
  (an external signal) are separate features. The HPF existed long before
  the key.
- Auto release ignores the `release` knob, as on the hardware it imitates.

## Known gaps

- Picking the key source has no GUI in the app; only the control API sets
  it (see [sidechain-routing.md](../mixer/sidechain-routing.md)).
