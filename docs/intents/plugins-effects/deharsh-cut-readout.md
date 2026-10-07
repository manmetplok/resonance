# De-harsh cut readout

> Status: **idea**
> Area: plugins-effects (mastering plugin editor), control-api (meter field)
> Source: `docs/history/specs/deharsh-resonance-suppressor.md` §7 open question 7; confirmed open in code on 2026-10-07.

## Why

The de-harsh stage of the mastering chain cuts narrow resonances that come
and go. Neither a user nor an agent can see what it is doing. The mastering
editor's De-harsh tab shows only knobs, and no meter field reports the cut.
The `delta` switch (listen to what is removed) helps a person with ears, but
an agent cannot hear. The skills tell agents to start de-harsh "at the
defaults" and then judge only by the broadband `meter_compare` deltas. They
cannot read "cutting 4.1 dB at 3.2 kHz" or notice that the stage is doing
nothing.

## What it would do

- The De-harsh tab draws the current cut curve (dB per frequency across the
  band) and a deepest-cut number, live.
- An agent can read the same thing: the deepest cut over a measured range,
  and where it was, either as a field of an existing meter result or
  through the plugin's params listing.
- `mixing/references/reading-meters.md` / `mastering/SKILL.md` explain the
  field.

## What exists to build on

- `resonance_dsp::deharsh::ResonanceSuppressor::max_cut_db()` and
  `cut_db(channel)` already compute it (`resonance-dsp/src/deharsh.rs`).
- `DeharshStage::max_cut_db` (`plugins/resonance-mastering/src/stages/deharsh.rs`)
  and `Chain::deharsh_max_cut_db` (`src/chain.rs`) expose it. Nothing calls
  the chain method, not even a test.
- The mastering viz/metering tap (`src/viz.rs`, `src/dsp.rs`) already
  publishes lock-free atomics for the editor. The multiband's per-band GR
  meter (`tests/multiband_band_gr.rs`) is the pattern for a published
  per-stage reduction.
- Feature doc: [deharsh.md](../../features/plugins-effects/deharsh.md).

## Open decisions

1. **Wire shape.** Recommended: a read-only param (`dh_cut_db`, like the
   drums' read-only `kit_load_progress`), so `master_plugin_params` returns
   it with no protocol change (`Param::is_read_only` exists). The
   alternative, a `meter.measure` field, would need the offline render to
   report a plugin's internal state, which nothing does today.
2. **Curve or number on the wire.** Recommended: number plus peak
   frequency on the wire, full curve only in the editor.
3. **Peak hold.** Recommended: deepest cut over the last ~1 s, decaying,
   so an agent polling after playback sees something.

## Slices

1. Editor: publish `max_cut_db` and the cut curve to the viz (RT-safe
   atomics / fixed ring), draw them in the De-harsh tab, headless editor
   test.
2. Wire: the read-only param (or meter field) + skill text + lockstep.

## How we'd know it works

- A test feeds the T1 resonance (Q 10, +15 dB) through the chain with
  de-harsh on and reads back a cut within 1 dB of what
  `resonance-dsp/tests/deharsh.rs` asserts. Broadband noise reads under
  0.5 dB.
- An agent session reports the cut before and after changing `dh_depth`.
