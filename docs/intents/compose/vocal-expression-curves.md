# Editable vocal expression curves (dock UI, persistence, control)

> Status: **partly built**
> Area: compose
> Source: prototype `docs/intents/prototypes/editable-vocal-expression-curves/index.html` (doc #154, epic #17; todos #333–#336); code comments citing todo #335.

## Why

Singers shape dynamics, tension, breathiness and pitch over a phrase. The
model and render feed for that exist, but nobody can reach them: no view
sends `ComposeMessage::Expression`, edits are not saved
(`load_from_project` clears `expression_curves` on open and on every
undo), and there is no control method. A rendered vocal always uses the
generated baseline.

## What it would do

As in the prototype: an Expression dock under the vocal roll with a curve
rail (Dynamics, Tension, Breathiness, Pitch bend; status auto / edited /
not supported by the voicebank), draw / points / line pens, snap to
syllables, depth over baseline, smoothing, reset to generated, and a
re-render on edit. Curves persist with the project and survive undo.
Agents can read and set breakpoints.

## What exists to build on

- Model: `resonance-app/src/compose/expression.rs` (`ExpressionCurves`,
  `ExpressionCurve::evaluate`, `CurveStatus`, depth/smoothing ranges),
  `expression_edit.rs` (`ExpressionDockState`, `PenMode`, `snap_time`).
- Handlers: `update/compose/expression.rs` (`ExpressionMessage` add / move
  / remove breakpoint, depth, smoothing, reset; requests re-render).
- Render feed: `compose/vocal_svs/segment/` reads the curves;
  `curve_supported` (TIGER lacks tension and breathiness).
- Tests: `tests/vocal/expression_curve_edits.rs`,
  `expression_curves_model.rs`, `vocal_expression_segment.rs`,
  `voicebank_curve_support.rs`.
- Feature doc: [vocal rendering](../../features/compose/vocal-rendering.md).

## Open decisions

1. Persistence home: inside `VocalParams` (rides lane config and undo for
   free — recommended) or a separate `ProjectFile` table keyed
   `(definition, track)`.
2. Control shape: one `vocal.set_curve` taking a full breakpoint list
   (recommended, idempotent) vs. per-breakpoint calls.

## Slices

1. Persistence + undo survival (model already serde); round-trip test.
2. Dock view (canvas + rail), goldens under `tests/snapshots/`.
3. `vocal.set_curve` / `song.vocal` curve readout, tests in
   `tests/control/`.

## How we'd know it works

Edit a dynamics curve, save, reopen, undo an unrelated edit: the curve is
still there and the re-rendered segment carries it
(`vocal_expression_segment` style assertion); goldens for each dock state.
