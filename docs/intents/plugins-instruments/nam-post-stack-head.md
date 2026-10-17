# NAM: load models with a top-level post-stack head

> Stage: **intent**
> Status: **idea**
> Area: plugins-instruments
> Touches: amp plugin
> Source: [plugin audit](../../history/reviews/plugin-audit.md) A1

## Why

Some NAM WaveNet exports end in a top-level head after the layer stacks.
The amp plugin's parser refuses any such head on an A2 config, and a
Windowed head outright, so those models fail to load with an error even
though they are valid upstream models. Only the per-stack windowed-head
rechannel (#1111) was built.

## What it would do

Models with a top-level post-stack head (including Windowed) load and
play, matching upstream NAM output to the existing parity tolerance.

## What exists to build on

- `plugins/resonance-amp/src/nam/parse/config.rs:178,207` — the refusals.
- The A2 parity harness and fixtures used for the A1/A2 work
  ([`../../features/plugins-instruments/amp-plugin.md`](../../features/plugins-instruments/amp-plugin.md)).

## Open decisions

1. Which head kinds to support first. **Recommended:** whatever the
   Tone3000 catalogue actually ships; survey before building.

## Slices

1. Parse + run the head; parity test against an upstream-rendered fixture.

## How we'd know it works

A parity test on a real model with a post-stack head, at the same ≤1e-6
tolerance as the existing A1/A2 tests.
