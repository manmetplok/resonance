# Pop/jazz cadence colour

> Status: **idea**
> Area: compose (`resonance-music-theory`)
> Source: `docs/history/specs/todo.md` Tier 4, "Pop/jazz cadence color" (§2C/§2E). Still open at `8eac71b3`: `CadenceGoal` (`derive/cadence.rs`) has only PAC/IAC/HC/Deceptive formulas; `SchemaKind` has no turnaround; the 12-bar blues is fixed `I I I I | IV IV I I | V IV I I`; schema variation is rotation + substitution only. The modal Markov table hints at an aeolian ♭VI→♭VII move, but there is no cadence for it.

## Why

Generated endings are always classical (2→1, 7→1, 3→2 …). Pop and jazz
songs close differently: the plagal "sigh" melody 6→♭6→5, aeolian
♭VI–♭VII–i, a ii–V in bars 9–10 of a blues, a I–vi–ii–V turnaround. Without
them, songs in those styles end like hymns.

## What it would do

- Cadence engine: a plagal-sigh melodic formula (6→♭6→5) chosen when the
  final chords are IV–iv–I.
- Harmony: an aeolian ♭VI–♭VII–i cadence option; a jazz ii–V substitution
  for bars 9–10 of `TwelveBarBlues`; a `Turnaround` (I–vi–ii–V) schema;
  a third schema variation operator, "cadence-aware final bars", next to
  rotation and substitution.

## What exists to build on

- `resonance-music-theory/src/derive/cadence.rs` (`CadenceGoal`,
  `formula_candidates`, fallback chains).
- `generator/schema.rs` (`SchemaKind`, `PlagalSigh` harmony already
  exists), `generator/table.rs` (modal / jazz tables).
- Tests: `cadence_formulas.rs`, `schema_generator.rs`.

## Open decisions

1. Chromatic formulas (♭6) need non-diatonic melody notes; allow them only
   when the final chords contain the borrowed tone (recommended).
2. Expose the new options in the chord inspector schema pick list
   (recommended) and in `harmony.apply_progression` presets.

## Slices

1. Turnaround schema + blues ii–V option; tests.
2. Cadence-aware final-bars operator.
3. Plagal-sigh melodic formula + aeolian cadence; tests.

## How we'd know it works

Schema tests show the new chord sequences; cadence tests show the 6→♭6→5
formula selected over IV–iv–I and never elsewhere.
