# Tempo step changes

> Status: **idea**
> Area: timeline (see `docs/features/timeline/tempo-and-signature-map.md`)
> Source: found while documenting the tempo map, 2026-10-07; follows from ba todo #1382 (one tempo event per bar)

## Why

Tempo ramps linearly between consecutive events (`bpm_at_bar`,
`avg_bpm_for_bar` in `resonance-audio/src/types/tempo/conversion.rs`). Adding
"140 BPM at bar 33" to a 120 BPM song makes bars 1–33 a slow accelerando,
not a jump at bar 33. Most songs want the jump. The map's math supports a
step (two events on one bar, last wins), but since ba todo #1382 both the
GUI and `global.add_tempo_event` upsert by bar, so a step can no longer be
made. Agents are not told this either: the `global.*` docs in
`resonance-control/src/methods/global.rs` speak of "a tempo change at a bar"
and say a removed event leaves the song "at whatever tempo the previous event
set", which is only true when there is no later event.

## What it would do

Each tempo event says how it is reached: `step` (jump at the bar; the
default) or `ramp` (linear from the previous event). The GUI tempo lane draws
steps as steps and offers a toggle on a selected point; `global.add_tempo_event`
/ `edit_tempo_event` take an optional `curve: "step" | "ramp"` and
`global.list_events` / `song.summary` report it.

## What exists to build on

- `TempoPoint { bar, bpm }` and the bar table already distinguish departure
  and arrival BPM (`BarEntry::{bpm, arrival_bpm}`, `arrival_bpm_at_bar`), which
  is how the old same-bar step worked.
- `tick_frac_to_sample_frac` already handles both flat and ramped bars.
- Tests for steps: `resonance-audio/tests/types/tempo_map.rs`
  (`bpm_at_bar_step_change_last_wins`, `tick_to_abs_sample_with_step_change`).

## Open decisions

1. Default for new events: **step** (recommended; matches what users and
   agents expect). Existing projects load as `ramp` to keep their timing.
2. Store as a field on `TempoPoint` (recommended, `#[serde(default)]`) vs.
   re-allowing two events per bar (rejected: breaks bar addressing in
   `global.*`).
3. Does the bar-0 event take a curve? No — nothing precedes it.

## Slices

1. Field on `TempoPoint` + map math + persistence + `resonance-audio` tests.
2. `global.*` params/views + MCP tool schema + control tests + skill text
   (`song-structure`).
3. GUI: draw steps, toggle on the selected point, golden.

## How we'd know it works

A 120→140 step at bar 33 puts bar 33 at exactly 32 bars of 120 BPM in
samples; `global.list_events` round-trips the curve; shelf golden shows the
step.
