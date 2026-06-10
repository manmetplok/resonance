# Melody & harmony generation improvements — 2026-06-10

From `melody-generation-research.md` (code audit + Open Music Theory v2).
Goal: fix "boring and samey" generated melodies. Ordered by suggested
implementation sequence; each item is independently testable.

## Tier 1 — melodic line quality (quick wins first)

- [x] **Well-formed-line quick wins in the motif builder** — `resonance-music-theory/src/derive/motif_engine/build.rs:72-95`: replace the 50/50 step-direction coin with step inertia (~0.62 continue same direction) plus melodic regression bias near register edges; cap consecutive repeated pitches at 2; forbid generated leaps of exactly 6 semitones (tritone) and >7. Add unit tests asserting the line rules (range ≤ 10th, no tritone/7th leaps).
- [x] **Leap recovery instead of gap-fill erasure** — `resonance-music-theory/src/derive/motif_engine/phrase.rs` (`apply_gap_fill`): a leap ≥ 4th should be *resolved* by a step in the opposite direction, not retroactively filled with passing tones. Also reject ≥3 consecutive leaps and same-direction leap pairs that don't outline one triad, and check direction-change extrema don't outline a tritone/7th.
- [ ] **Single-climax enforcement per phrase** — `motif_engine/phrase.rs:155-171` + vocal `derive/vocal/melody.rs:32-42`: exactly one highest note per phrase, in its second half, not the final note, ideally approached by leap; demote duplicate peaks (wave contour currently produces two). Secondary lower peaks allowed in long phrases.
- [ ] **Cadence formula targeting (instrument + vocal)** — give every phrase a goal cadence (HC/IAC/PAC; weak for antecedent, strong for consequent) and force the final two melody notes to a formula compatible with the chord: 2→1 / 7→1 (PAC), ends-on-3/5 (IAC), 1→7 / 3→2 (HC). Replace the fixed vocal degree sets in `derive/vocal/style/mod.rs:233-299` (`cadence_pitch`) with the same formula table plus ~10% deceptive endings; resolve tendency tones 7→1, 4→3, 2→1.
- [ ] **Phrase grammar: sentence/period/srdc planner** — `motif_engine/phrase.rs` (`plan_motif_transforms`, `realize_phrase`): replace independent per-phrase transform draws with phrase plans. Sentence: basic idea (2 bars) + varied repeat (presentation, no cadence) → continuation with fragmentation of the idea's head motive, denser surface rhythm, faster harmonic rhythm, cadence. Period: consequent reuses the antecedent's opening, swaps the ending weak→strong. Pop srdc (aaba/aabc) for section-level vocal layout.
- [ ] **Embellishing-tone decoration pass** — generate a chord-tone skeleton on strong beats, then decorate from the OMT table (passing/neighbor on weak beats; appoggiatura/suspension on strong beats; escape tone, anticipation) with style-weighted probabilities, replacing blanket chord/scale snapping in `align_to_harmony`. Constraint: never leap both into and out of a dissonance.
- [ ] **Rhythm transforms** — `motif_engine/build.rs:15-24`: keep the 8 base patterns but add the straight-syncopation transform (halve first note, shift the rest earlier; 8th or 16th level), tresillo cells (3+3+2 and rotations, double tresillo), and rhythmic acceleration in continuations (halve note values when fragmenting). Vocals: allow division-level syncopation on stressed syllables instead of pure grid + ±0.08 jitter.

## Tier 2 — harmony

- [ ] **Pop schema bank as a new generator** — add `GeneratorSpec::Schema` alongside the Markov sampler (`resonance-music-theory/src/generator/`): 12-bar blues, doo-wop I–vi–IV–V, axis I–V–vi–IV (any rotation), hopscotch IV–V–vi–I, lament i–♭VII–♭VI–V, plagal family (I–IV vamp, ♭VII–IV–I, plagal sigh IV–iv–I), modal shuttles (I–♭VII, i–IV, I–II♯), circle of fifths, puff I–iii–IV opener. Variation via rotation and function-preserving substitution (swap chords sharing ≥2 tones).
- [ ] **Phrase-model overlay on Markov output** — `generator/markov.rs` + `table.rs`: tag degrees with T/PD/D function per table and constrain sampled progressions to one T→PD→D→(T) arc per phrase; accelerate harmonic rhythm into bars 4/8; place cadential dominants on hyper-downbeats of 4-bar groups.

## Tier 3 — structure & variation depth

- [ ] **Section-level climax orchestration** — one climax per *section*, placed in a designated phrase (e.g. phrase 3 of 4), other phrases get lower secondary peaks; replaces independent per-phrase/per-line contour draws (fixes every vocal line arching identically).
- [ ] **Sequences as a transform** — add real sequences (model + transposed copies: descending fifths, descending thirds, ascending 5–6) to the transform vocabulary in `plan_motif_transforms`, used in continuations/departures.
- [ ] **Composable transforms** — allow transform pairs (fragment+transpose, invert+augment) at high complexity in `motif_engine/phrase.rs:92-151`; widen the operator vocabulary rather than the randomness.
