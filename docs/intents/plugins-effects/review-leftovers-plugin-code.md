# First-party plugins: DSP and editor leftovers

> Status: **idea** (checklist)
> Area: plugins-effects (also covers instrument plugins)
> Source: [code review 2026-10-02](../../history/reviews/code-review-2026-10-02.md) FU-D1a3, FU-D1a4, FU-D1b2, FU-D1b4, FU-D1b5, FU-P2d, FU-P3a, FU-P3b, FU-P3c, FU-U3a, FU-X1a

## Why

Performance, consistency and hygiene items in the first-party plugins that
the 2026-10-02 campaign left open. Audible or visible plugin defects are in
[`../bugs.md`](../bugs.md) (BUG-41, BUG-42).

## What it would do

Effects:

- [ ] **FU-D1a3** — the linear-phase EQ keeps an inline last-resort FIR
      design on its worker (`plugins/resonance-mastering/src/stages/linear_phase_eq/worker.rs:339,399`);
      remove once the precomputed path covers every case.
- [ ] **FU-D1a4** *(likely)* — linear-phase EQ requests made during a
      crossfade wait for the whole fade; queue the latest only.
- [ ] **FU-D1b4** — the granular golden is only re-blessable with
      `RESONANCE_BLESS_GRANULAR_DSP`, not the standard `RESONANCE_BLESS`
      (`plugins/resonance-granular-delay/tests/dsp_regression.rs:568`).
- [ ] **FU-X1a** — the mastering reference loader has its own symphonia
      decode loop (`plugins/resonance-mastering/src/assistant/reference.rs:1-20`);
      use the shared decoder.

Editors:

- [ ] **FU-P3a** — 12 editors request a 16/33 ms repaint every frame; add
      idle tiers (e.g. `plugins/resonance-color/src/editor/app.rs:44`).
- [ ] **FU-P3b** — `Vec<&dyn Param>` rebuilt every frame in 8 editors
      (e.g. `plugins/resonance-eq/src/editor/app.rs:113`).
- [ ] **FU-U3a** — dead `ui-work.md §11` references
      (`plugins/resonance-eq/src/editor/app.rs:50`,
      `plugins/resonance-color/src/editor/mod.rs:16`).

Instruments:

- [ ] **FU-D1b2** — profile the drums band-limited sample read; add
      decimated copies if it shows up.
- [ ] **FU-D1b5** — wavetable envelope coefficients are computed once per
      block, not per sample/segment (`plugins/resonance-wavetable/src/dsp/render/plan.rs:252-260`).
- [ ] **FU-P2d** — drums knobs have no typed entry (`plugins/resonance-drums/src/editor/controls.rs:268-285`).
- [ ] **FU-P3c** — drums `live_value` covers only 2 params (`plugins/resonance-drums/src/lib.rs:1013-1019`).

## What exists to build on

[`../../features/plugins-instruments/plugin-editor-runtimes.md`](../../features/plugins-instruments/plugin-editor-runtimes.md),
[`../../features/plugins-instruments/dsp-test-support.md`](../../features/plugins-instruments/dsp-test-support.md),
[`plugin-editor-visual-checks.md`](plugin-editor-visual-checks.md).

## Open decisions

None.

## Slices

One commit per checkbox.

## How we'd know it works

Goldens unchanged (or re-blessed deliberately); a CPU measurement for the
repaint tiers with an idle editor open.
