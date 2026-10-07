# Live agent session with the mix-character effects

> Status: **partly built** (everything is built; the end-to-end check is not done)
> Area: plugins-effects (with the skills in `resonance-agent-plugin/`)
> Source: `docs/history/specs/reverb-algorithms.md` status line and phase R7 exit ("one live agent session that mixes a project end to end using the type table"); `docs/history/specs/warmth-width-depth.md` slice W4 exit and the build's open items (live agent session). Still open on 2026-10-07: no session report exists in the repo.

## Why

The reverb types, Color, Stereo, the EQ's warmth bands and dynamic bands,
the compressor's Auto release and the mastering extensions were all built so
an agent can make warmth, width and depth moves by number. Every piece is
tested in isolation: goldens, harmonic signatures, decay acceptance numbers,
skill-key lockstep. Nobody has run the whole loop. An agent following the
`mixing`, `spatial` and `mastering` skills has not mixed a real project in
the running app and reported its moves. So nothing yet shows that the skills
choose sensible types and presets, that the meters move the way the skills
predict, or that the result sounds right to a person.

## What it would do

One recorded session per skill family on a real project (or one session
covering all three):

- `spatial`: reverb returns picked from the type table, decay set from
  tempo or `decay_sync` and checked with `meter_measure` `detail:
  ["decay"]`; width with Stereo; delay throws.
- `mixing`: de-harsh, warmth with Color on busses (drive set by
  `meter_probe`), tilt with the EQ, bus glue with the compressor, each move
  kept or dropped by `meter_compare`.
- `mastering`: stages enabled one at a time, `master_assist` read and
  applied selectively.

The session's moves and before/after numbers are written up. Jorrit
listens to the result, and any skill text that led the agent wrong is
fixed.

## What exists to build on

- The plugins: [reverb](../../features/plugins-effects/reverb.md),
  [color](../../features/plugins-effects/color.md),
  [stereo](../../features/plugins-effects/stereo.md),
  [eq](../../features/plugins-effects/eq.md),
  [compressor](../../features/plugins-effects/compressor.md),
  [delay](../../features/plugins-effects/delay.md),
  [mastering](../../features/plugins-effects/mastering.md).
- The skills and their lockstep tests (`resonance-mcp/tests/agent_plugin_lockstep.rs`,
  each plugin's `tests/skill_keys.rs`).
- The meters the skills read: `meter.measure` / `meter.stems` details
  (spectrum, stereo, dynamics, depth, decay), `meter.snapshot`,
  `meter.compare`, `meter.probe`, `master.assist`.

## Open decisions

1. **Which project.** Recommended: a real multitrack Jorrit knows well,
   not a generated one, so "does it sound better" has an answer.
2. **Where the report goes.** Recommended: `docs/history/reviews/` as a
   field report, with skill fixes landing as normal commits.

## Slices

1. Spatial session (reverb types + decay check), the R7 exit.
2. Mixing/character session (Color, EQ, compressor), the W4 exit.
3. Mastering session (chain + assistant).

## How we'd know it works

- Each session's report lists every move with the meter numbers that
  justified it.
- Jorrit's listening verdict is recorded.
- Skill changes from it pass `agent_plugin_lockstep` and `skill_keys`.
