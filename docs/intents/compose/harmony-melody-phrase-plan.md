# Share the phrase plan between harmony and melody

> Status: **idea**
> Area: compose (`resonance-music-theory`)
> Source: `docs/history/specs/todo.md` Tier 4, "Harmony ↔ melody phrase-plan coordination" (§2B of `melody-generation-research.md`; deferred in f676696). Still open at `8eac71b3`: `phrase_grammar_roles` is only referenced inside `derive/motif_engine/`, never in `generator/`.

## Why

The melody engine plans sentences and periods (basic idea, varied repeat,
continuation, cadence) and the Markov progression generator has its own
phrase-model overlay (T→PD→D arc, faster harmonic rhythm into bars 4/8).
Both think in 4-bar groups but never talk, so a presentation phrase can
get a busy, cadential progression while its melody repeats the idea, and
a continuation's melodic acceleration can sit over static harmony. The
result is less coherent than either half alone.

## What it would do

One section-level form plan drives both: presentation phrases get
tonic-prolonging harmony, continuations get the faster harmonic rhythm and
the cadential approach, and the cadence types the melody targets match
the chords under them.

## What exists to build on

- `resonance-music-theory/src/derive/motif_engine/phrase.rs`
  (`phrase_grammar_roles`, `PhraseGrammarRole`).
- `resonance-music-theory/src/generator/markov/{phrase,cadence,fill}.rs`
  (overlay, harmonic-rhythm splits), `GenContext`.
- Feature docs: [melody and motif engine](../../features/compose/melody-and-motif-engine.md),
  [section chords](../../features/compose/section-chords-and-progressions.md).

## Open decisions

1. Who owns the plan: a new `FormPlan` computed once per section from its
   length and seed, passed to both generators (recommended).
2. Store it on the section so regenerating one side keeps the other
   aligned? Recommended: yes, in `generated_material`.

## Slices

1. `FormPlan` type + derivation; melody engine reads it instead of
   computing roles itself (no behaviour change; goldens hold).
2. Markov overlay reads it; new tests asserting tonic prolongation in
   presentations and splits in continuations.

## How we'd know it works

Seeded tests in `resonance-music-theory/tests/` that, for the same plan,
presentation bars carry tonic-function chords and the cadence bar's chord
matches the melody's cadence goal; `markov_golden.rs` re-blessed
deliberately.
