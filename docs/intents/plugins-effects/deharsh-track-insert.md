# De-harsh on tracks and busses

> Stage: **intent**
> Status: **idea**
> Area: plugins-effects
> Source: `docs/history/specs/deharsh-resonance-suppressor.md` §5.3 and §7 open question 8; `docs/history/specs/warmth-width-depth.md` §6.3 ("the suppressor core lives in `resonance-dsp` so the `eq` plugin or a track-level insert can reuse it later"). Confirmed not built on 2026-10-07.

## Why

Harshness usually starts on a track (a bright vocal, distorted guitars,
cymbals), not on the master. The resonance suppressor exists only as a stage
of the mastering chain. On a track or bus, the `mixing` skill's "De-harsh
first" step has to use an EQ dynamic bell at 0 dB. That works for one known
resonance, not for many narrow ones that move with the notes, which is what
a suppressor is for.

## What it would do

A user or agent can put the suppressor on any track or bus, with the same
controls as the mastering stage (depth, selectivity, sharpness, attack,
release, band, mode, mix, delta). It reports a constant latency, and the
skills can name its keys in a keys block.

## What exists to build on

- `resonance_dsp::deharsh::ResonanceSuppressor` with `process_stereo` and
  `process_mono`, `SuppressorConfig`, the `*_RANGE` constants for hosts
  (`resonance-dsp/src/deharsh.rs`). It has no mastering dependency.
- The mastering wrapper and its params to copy:
  `plugins/resonance-mastering/src/stages/deharsh.rs`,
  `src/params/deharsh.rs`, `src/editor/controls/deharsh.rs`.
- The EQ's key port and dynamic-band machinery
  (`plugins/resonance-eq/src/dsp.rs`) if it becomes an EQ mode.
- Feature docs: [deharsh.md](../../features/plugins-effects/deharsh.md),
  [eq.md](../../features/plugins-effects/eq.md).

## Open decisions

1. **Where it lives.** Options: a new `resonance-deharsh` plugin, a mode or
   band type of `resonance-eq`, or a stage in `resonance-color`.
   Recommended: a small new plugin. The EQ is zero-latency today and
   should stay so; adding a 43 ms STFT frame to it would charge every EQ
   instance. The plugin-audit plan's "no sprawl of single-trick plugins"
   argues the other way, so this needs a call.
2. **Latency.** 2048 samples (≈ 43 ms at 48 kHz) is fine on a master and
   costly while tracking. Recommended: offer the design note's
   low-latency geometry (a shorter frame) as a choice, applied via
   `HostHandle::set_latency_samples` and a host restart, as the IR plugin's
   latency mode does.
3. **Sidechain key.** Design note v1.1 candidate. Recommended: not in the
   first slice.

## Slices

1. Plugin (or EQ mode) with the `dh_*`-equivalent params, editor panel,
   factory presets (vocal, cymbals, guitar bus), golden, state, skill_keys.
2. Skill: replace or complement the dynamic-bell recipe in
   `mixing/SKILL.md` and `references/character.md`.
3. Optional: low-latency geometry choice.

## How we'd know it works

- The W12 exit criteria hold on a track insert: a synthetic 3.2 kHz
  resonance cut by at least 6 dB, broadband change under 0.5 dB, constant
  reported latency.
- An agent mixing session uses it on a harsh vocal and `meter_compare`
  shows `presence_peakiness_db` falling with matched loudness.
