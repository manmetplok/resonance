# Melody and motif engine

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/melody-generation-research.md` (diagnosis + Open Music Theory principles), `docs/history/specs/todo.md` (implementation checklist; 17 of 22 items done).

The melody generators turn a section's chords into lines that behave like
written melodies: a shared motif that is varied across phrases, phrase
forms (sentence, period, srdc), goal cadences, one climax, recovered
leaps, embellishing tones and rhythm variation. They drive the Motif-style
lead and bass lanes, the arp styles, and (with their own vocal profiles)
the vocal melody. This was the fix for "boring and samey" output.

## Behaviour

- **Motif source.** Each section owns a `MotifSource`: `Generated(
  MotifParams { seed, complexity, motif_len, leap_chance })` or a
  hand-drawn manual motif (scale-step × 16th grid cells, rests, durations,
  accents). `motif_intervals` gives the intervals other lanes (vocal
  `use_section_motif`, drum Motif voices) reuse.
- **Building the cell** (`motif_engine/build.rs`): eight base rhythm
  patterns plus transforms — straight syncopation (8th / 16th),
  tresillo cells and rotations, double tresillo, acceleration in
  continuations. Pitch walk with step inertia (~0.62 keep direction),
  regression near register edges, a soft descending-step bias (Huron), at
  most two repeated pitches in a row, no generated tritone leaps or leaps
  over a fifth.
- **Phrase plan** (`phrase.rs`): `phrase_grammar_roles` assigns
  `PhraseGrammarRole` (BasicIdea, VariedRepeat, Continuation,
  ContinuationCadence, Antecedent, Consequent) for sentence / period forms;
  `plan_motif_transforms` draws per-phrase `Transform`s — Identity,
  TransposeUp/Down, Invert, Retrograde, Augment, Diminish, Fragment,
  Syncopate, `Sequence` (`SequenceKind` DescendingFifths, DescendingThirds,
  Ascending56) and `Composed` pairs (FragmentTranspose, InvertAugment,
  RetrogradeInvert) at higher complexity.
- **Harmony alignment** (`harmony.rs`, `embellish.rs`): a chord-tone
  skeleton on strong beats, then decoration with passing, neighbour,
  appoggiatura, suspension, escape and anticipation tones weighted by
  `EmbellishmentStyle` (Auto, Folk, PopBallad, Jazz); never leap both into
  and out of a dissonance. Leaps of a fourth or more are recovered by a
  step the other way; three leaps in a row and same-direction leap pairs
  that do not outline a triad are rejected.
- **Cadences** (`derive/cadence.rs`, `motif_engine/cadence.rs`): each
  phrase gets a `CadenceGoal` — antecedents HC (sometimes IAC), consequents
  PAC with ~10% deceptive — and its last two notes are forced to a formula
  that fits the final chord: PAC 2→1 / 7→1; IAC 4→3, 2→3, 6→5, 4→5; HC
  1→7, 3→2; deceptive 7→6, 5→6, with a fallback chain.
- **Climax** (`derive/climax.rs`): exactly one highest note per phrase, in
  its second half, not the last note; at section level one designated
  phrase (`section_climax_phrase`) holds the section climax and the others
  get lower peaks. This runs per lane; lanes are not coordinated.
- **Vocal profiles.** The vocal generator (`derive/vocal/style/`) has six
  style profiles and reuses cadence formulas, section climax and an srdc
  (aaba / aabc) layout — see
  [vocal lyrics and melody](vocal-lyrics-and-melody.md).
- **Bass.** `derive_bass` styles plus `derive_bass_motif` (motif-locked
  bass with `BassMotifMode` / `BassMotifPhrase`).
- **Pad.** `derive_pad` voices chords through `satb_voicings`.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Motif source | `resonance-music-theory/src/derive/motif_source.rs`, `motif_rhythm.rs` | `MotifSource`, `MotifParams`, `ManualMotifCell`, `toggle_manual_motif_cell`, `derive_motif_rhythm` |
| Engine | `src/derive/motif_engine/{build,phrase,harmony,embellish,cadence,melody,types}.rs` | `choose_direction`, `plan_motif_transforms`, `phrase_grammar_roles`, `section_climax_phrase`, `derive_motif_melody_with_section`, `Transform` |
| Cadence + climax | `src/derive/cadence.rs`, `climax.rs` | `CadenceGoal`, `plan_cadence_goal`, `formula_candidates`, `enforce_single_climax` |
| Non-motif melody | `src/derive/melody.rs` | `MelodyParams`, `MelodyStyle`, `derive_melody`, `derive_melody_fill_vocal`, `EmbellishmentStyle` |
| Bass / pad | `src/derive/{bass,motif_bass,pad}.rs` | `derive_bass`, `derive_bass_motif`, `derive_pad` |
| App bridge | `resonance-app/src/compose/generate.rs` | `derive_notes` |
| Motif UI | `resonance-app/src/view/compose/lane_inspector/chord/motif.rs`, `manual_motif_canvas.rs` | complexity / length / leap chance, manual grid |

## Surfaces

- **GUI:** chord inspector motif group (shared motif, Regenerate motif,
  Generated vs Manual source and the manual grid); lane inspector melody
  and bass params (style, contour, register, …). No control for
  `EmbellishmentStyle`.
- **Control API / MCP:** all `MelodyParams` / `BassParams` fields,
  `embellishment` included, can be passed as `options` to `generate.part`
  or `section.set_lane_generator`. The section motif has no control
  method.
- **Agent skills:** `arranging` (generate_part options).

## How it's tested

See the "Motif engine" and "Vocal" rows in
[music-theory library](music-theory-library.md#how-its-tested); all run
with `./scripts/run-tests.py -p resonance-music-theory`. Plan-level
properties (sequence rates, copy counts) are asserted on
`plan_motif_transforms` directly because repair passes hide them in the
notes.

Not covered: audible effect of `Augment` (see below); taste.

## Decisions and deviations

- The research's tier 1–3 and most tier 4 items are built (see
  `docs/history/specs/todo.md`). The phrase planner and the Markov phrase
  overlay both work in 4-bar groups but do not share a plan.
- `Augment` doubles each `duration_ratio`, but the tiling realizer
  normalises ratios per chord (`phrase.rs`, `total_ratio`), so uniform
  augmentation mostly disappears at render time. It is still drawn.

## Known gaps

- [harmony-melody-phrase-plan](../../intents/compose/harmony-melody-phrase-plan.md)
- [pop-jazz-cadence-color](../../intents/compose/pop-jazz-cadence-color.md)
- [cross-lane-climax-separation](../../intents/compose/cross-lane-climax-separation.md)
- [embellishment-style-ui](../../intents/compose/embellishment-style-ui.md)
- [motif-augment-washout](../../intents/compose/motif-augment-washout.md)
