# Visual checks for effect plugin editors

> Stage: **intent**
> Status: **idea**
> Area: plugins-effects
> Source: `docs/history/specs/reverb-algorithms.md` status line ("open: … the by-hand Wayland editor tests") and §4.6; the warmth-width-depth build's open item "editor visuals" (Color, Stereo). Confirmed on 2026-10-07: no plugin crate has a pixel snapshot test.

## Why

Every effect plugin's editor is tested headless: layout tables, knob ↔ param
binding, "fits its window" checks, viz data round trips. None of that sees
what is drawn. The by-hand Wayland/Cocoa tests (`editor_open`,
`editor_size`, `editor_close`) check the editor lifecycle, not its looks.
The newest views have never been looked at systematically: the reverb's
per-algorithm tank views, Color's transfer curve and harmonic bars, and
Stereo's goniometer and mono-risk pill. A broken plot would go unnoticed
until someone opened the window. The app's views have golden PNG snapshots
(`resonance-app/tests/snapshots`); the plugin editors have none.

## What it would do

Each effect plugin editor gets at least one golden image at its minimum
size with a fixed viz state (and one per algorithm for the reverb), rendered
headless and compared like the app's goldens, re-blessed with
`RESONANCE_BLESS=1`. Until that exists, a by-hand pass in a Wayland
session over the reverb (all nine tank views), Color and Stereo is recorded
as done.

## What exists to build on

- Headless editor frames already run in tests: e.g.
  `plugins/resonance-delay/tests/editor_layout.rs` (CPU-only frames of the
  real editor app), `plugins/resonance-reverb/tests/algorithms/editor.rs`
  (lays out and draws every algorithm without panicking).
- `plugin-gui-core` (egui) and `egui_glow`; the app's iced golden harness
  for the compare/bless pattern.
- `resonance-dsp-test-support` for golden IO and the bless switch.
- Memory note: this machine is canonical for goldens.

## Open decisions

1. **Renderer for goldens.** egui needs a painter to rasterise. Options: a
   CPU rasteriser of egui's tessellated meshes in a test-support crate, or
   a headless GL context. Recommended: CPU tessellation + a small software
   rasteriser, so the goldens need no GPU and stay deterministic.
2. **Scope.** Recommended: start with reverb, Color and Stereo (the open
   items), then the rest.
3. **Fonts.** Pin the font atlas in the test so text renders the same on
   every run.

## Slices

1. Golden harness for plugin editors in a shared test-support crate, plus
   one golden for Stereo.
2. Reverb: one golden per algorithm's tank view.
3. Color, then the remaining effect editors.

## How we'd know it works

- A deliberate one-pixel change to a plot fails its golden.
- The goldens hold across runs on this machine, and with
  `RESONANCE_RENDER_THREADS=8`.
