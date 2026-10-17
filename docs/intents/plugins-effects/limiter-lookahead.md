# Mastering limiter: adjustable lookahead

> Stage: **intent**
> Status: **idea**
> Area: plugins-effects
> Source: [plugin audit](../../history/reviews/plugin-audit.md) M6

## Why

The mastering plugin's limiter has a fixed 5 ms lookahead. That is a sound
default, but it is the main knob for the limiter's transient character:
shorter keeps punch on drums, longer is cleaner on dense material. Every
comparable mastering limiter exposes it, and an agent mastering a track
cannot trade punch for transparency without it.

## What it would do

A `Lookahead` parameter on the limiter stage (e.g. 1–10 ms, default 5),
in the editor's limiter section and in `master_plugin_params`, with the
plugin's reported latency following it.

## What exists to build on

- `plugins/resonance-mastering/src/stages/limiter.rs:47` — the constant.
- Host latency changes are handled live (two-stage PDC,
  [`../../features/audio-engine/latency-compensation.md`](../../features/audio-engine/latency-compensation.md)).
- Feature doc: [`../../features/plugins-effects/mastering.md`](../../features/plugins-effects/mastering.md).

## Open decisions

1. Allow automation of lookahead (each change is a latency change).
   **Recommended:** no — mark it non-automatable; latency changes
   mid-song cause PDC re-sync.
2. Range. **Recommended:** 1–10 ms, stepped in 0.5 ms.

## Slices

1. Param + state migration (default 5 ms keeps old sessions identical) +
   latency report + editor control + skill param key.

## How we'd know it works

DSP golden at the default still matches; new goldens at 1 ms and 10 ms;
a host test that the reported latency follows the param.
