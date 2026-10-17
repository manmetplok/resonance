# Readouts an agent can't see yet

> Stage: **intent**
> Status: **idea**
> Area: control-api
> Touches: with mixer metering and compose
> Source: [plugin audit plan](../../history/reviews/plugin-audit-plan.md) ledger "per-band GR, key-connected → meter.* readout" and X5;
> [code review 2026-09-26](../../history/reviews/code-review-todo.md) FU-M11b (rest);
> [field report 2026-10-06](../../history/reviews/field-report-2026-10-06.md) §4 follow-up

## Why

An agent cannot hear, so anything the GUI shows as a warning or a meter
and the control API does not return is invisible to it. Four such gaps
remain, each found by a different review:

- **Vocal tempo mismatch.** The Compose view warns when a section's single
  vocal render does not match a placement's tempo; `song.vocal` does not
  report it, so an agent ships an off-tempo vocal.
- **Dynamics state.** Compressor/gate gain reduction, "key connected", and
  mastering per-band GR are drawn in the editors only. An agent wiring a
  sidechain cannot confirm the key arrives or how hard it ducks.
- **Plugin latency.** Host latency tracking is correct, but MCP plugin
  entries have no `latency_samples` (the GUI readout is in
  [`../plugins-instruments/third-party-hosting-gaps.md`](../plugins-instruments/third-party-hosting-gaps.md)).
- **Reverb decay on dry-heavy mixes.** The `decay` meter fit starts at
  the dry cliff, so a mostly-dry return reads a misleading T30; the
  field report suggested fitting after the cliff or exposing
  `mid_t30_seconds`.

## What it would do

- `song.vocal` sections carry a `tempo_mismatch` flag (placement ids and
  the two tempos).
- Read-only output values for GR / key-connected on compressor, gate and
  mastering bands, readable through `*_plugin_params` or a `meter.probe`
  detail.
- `latency_samples` on every plugin entry in `song.tracks` / `*_plugin_params`.
- The decay detail reports a post-cliff fit (or `mid_t30_seconds`).

## What exists to build on

- `vocal_tempo_mismatch` in `resonance-app/src/update/compose/mod.rs:205`;
  `resonance-control/src/methods/vocal.rs`.
- `resonance-control/src/methods/meter.rs`, `meter.probe` and the
  `resonance_metering::decay` harness
  ([`../../features/mixer/meter-control-api.md`](../../features/mixer/meter-control-api.md)).
- Plugin-reported latency is already in the engine
  ([`../../features/audio-engine/latency-compensation.md`](../../features/audio-engine/latency-compensation.md)).
- The same pattern for de-harsh: [`../plugins-effects/deharsh-cut-readout.md`](../plugins-effects/deharsh-cut-readout.md).

## Open decisions

1. GR as CLAP output params vs a private extension. **Recommended:**
   read-only output params — third-party plugins use the same route.
2. One method per readout or fields on existing reads. **Recommended:**
   fields on existing reads; no new tools for the vocal and latency cases.

## Slices

1. `song.vocal` tempo mismatch (wire + handler + MCP + skill note).
2. `latency_samples` on plugin entries.
3. Compressor/gate GR + key-connected; then mastering per-band GR.
4. Decay post-cliff fit.

## How we'd know it works

Control-group tests per field; a sidechain test where an agent-visible GR
value goes non-zero only when the key track plays.
