# Melody & harmony generation improvements — 2026-06-10

From `melody-generation-research.md` (code audit + Open Music Theory v2).
Goal: fix "boring and samey" generated melodies. Ordered by suggested
implementation sequence; each item is independently testable.

## Tier 1 — melodic line quality (quick wins first)

- [x] **Well-formed-line quick wins in the motif builder** — `resonance-music-theory/src/derive/motif_engine/build.rs:72-95`: replace the 50/50 step-direction coin with step inertia (~0.62 continue same direction) plus melodic regression bias near register edges; cap consecutive repeated pitches at 2; forbid generated leaps of exactly 6 semitones (tritone) and >7. Add unit tests asserting the line rules (range ≤ 10th, no tritone/7th leaps).
- [x] **Leap recovery instead of gap-fill erasure** — `resonance-music-theory/src/derive/motif_engine/phrase.rs` (`apply_gap_fill`): a leap ≥ 4th should be *resolved* by a step in the opposite direction, not retroactively filled with passing tones. Also reject ≥3 consecutive leaps and same-direction leap pairs that don't outline one triad, and check direction-change extrema don't outline a tritone/7th.
- [x] **Single-climax enforcement per phrase** — `motif_engine/phrase.rs:155-171` + vocal `derive/vocal/melody.rs:32-42`: exactly one highest note per phrase, in its second half, not the final note, ideally approached by leap; demote duplicate peaks (wave contour currently produces two). Secondary lower peaks allowed in long phrases.
- [x] **Cadence formula targeting (instrument + vocal)** — give every phrase a goal cadence (HC/IAC/PAC; weak for antecedent, strong for consequent) and force the final two melody notes to a formula compatible with the chord: 2→1 / 7→1 (PAC), ends-on-3/5 (IAC), 1→7 / 3→2 (HC). Replace the fixed vocal degree sets in `derive/vocal/style/mod.rs:233-299` (`cadence_pitch`) with the same formula table plus ~10% deceptive endings; resolve tendency tones 7→1, 4→3, 2→1.
- [x] **Phrase grammar: sentence/period/srdc planner** — `motif_engine/phrase.rs` (`plan_motif_transforms`, `realize_phrase`): replace independent per-phrase transform draws with phrase plans. Sentence: basic idea (2 bars) + varied repeat (presentation, no cadence) → continuation with fragmentation of the idea's head motive, denser surface rhythm, faster harmonic rhythm, cadence. Period: consequent reuses the antecedent's opening, swaps the ending weak→strong. Pop srdc (aaba/aabc) for section-level vocal layout.
- [x] **Embellishing-tone decoration pass** — generate a chord-tone skeleton on strong beats, then decorate from the OMT table (passing/neighbor on weak beats; appoggiatura/suspension on strong beats; escape tone, anticipation) with style-weighted probabilities, replacing blanket chord/scale snapping in `align_to_harmony`. Constraint: never leap both into and out of a dissonance.
- [x] **Rhythm transforms** — `motif_engine/build.rs:15-24`: keep the 8 base patterns but add the straight-syncopation transform (halve first note, shift the rest earlier; 8th or 16th level), tresillo cells (3+3+2 and rotations, double tresillo), and rhythmic acceleration in continuations (halve note values when fragmenting). Vocals: allow division-level syncopation on stressed syllables instead of pure grid + ±0.08 jitter.

## Tier 2 — harmony

- [x] **Pop schema bank as a new generator** — add `GeneratorSpec::Schema` alongside the Markov sampler (`resonance-music-theory/src/generator/`): 12-bar blues, doo-wop I–vi–IV–V, axis I–V–vi–IV (any rotation), hopscotch IV–V–vi–I, lament i–♭VII–♭VI–V, plagal family (I–IV vamp, ♭VII–IV–I, plagal sigh IV–iv–I), modal shuttles (I–♭VII, i–IV, I–II♯), circle of fifths, puff I–iii–IV opener. Variation via rotation and function-preserving substitution (swap chords sharing ≥2 tones).
- [x] **UI wiring for the schema generator** *(follow-up added after `a89312e`)* — the chord lane inspector can't create or edit a `GeneratorSpec::Schema` yet: add a generator-type selector (Markov table vs. schema), a schema pick_list (`SchemaKind::ALL`/`name()`), and rotation/substitution controls; `ChordInspectorMsg` needs corresponding messages. Files: `resonance-app/src/view/.../chord_inspector.rs`, `lane_inspector/chord/*`. MUST route through the `ux-design` agent first, then `e2e-tester` for snapshot verification.
- [x] **Phrase-model overlay on Markov output** — `generator/markov.rs` + `table.rs`: tag degrees with T/PD/D function per table and constrain sampled progressions to one T→PD→D→(T) arc per phrase; accelerate harmonic rhythm into bars 4/8; place cadential dominants on hyper-downbeats of 4-bar groups.

## Tier 3 — structure & variation depth

- [x] **Section-level climax orchestration** — one climax per *section*, placed in a designated phrase (e.g. phrase 3 of 4), other phrases get lower secondary peaks; replaces independent per-phrase/per-line contour draws (fixes every vocal line arching identically).
- [x] **Sequences as a transform** — add real sequences (model + transposed copies: descending fifths, descending thirds, ascending 5–6) to the transform vocabulary in `plan_motif_transforms`, used in continuations/departures.
- [x] **Composable transforms** — allow transform pairs (fragment+transpose, invert+augment) at high complexity in `motif_engine/phrase.rs:92-151`; widen the operator vocabulary rather than the randomness.

## Tier 4 — remaining research items (added 2026-06-11, from `melody-generation-research.md` §2 leftovers)

- [x] **SATB-style voice leading for harmony rendering** — §2G, the only research section with zero implementation. Render chord lanes as voiced parts instead of block chords: bass first, melody built backwards from the cadence with correct tendency tones, inner voices to nearest chord tones; forbid parallel 5ths/octaves; never double the leading tone or a chordal 7th; prefer contrary motion against a rising 4→5 bass. New pass in `resonance-music-theory` consumed by the chord-lane renderer.
- [x] **Inversions + classical cadence machinery** — §2C: the harmony model has no inversions, so pre-dominant bass idioms (ii6, bass 4→5 via ii6/IV, IV-precedes-ii) and the cadential 6/4 (over bass 5, resolve 6→5 and 4→3 in the same voices) are inexpressible. Add inversion support to the chord model/generator, then the cadential 6/4 as a cadence-slot decoration. Builds on the voice-leading pass above.
- [ ] **Harmony ↔ melody phrase-plan coordination** — §2B: the sentence/period planner (`phrase_grammar_roles`) and the Markov phrase overlay both use 4-bar groups but don't share a plan. Share the section's form plan with the chord generator so presentations get tonic-prolonging harmony and continuations get the faster harmonic rhythm (the piece deferred in f676696).
- [ ] **Pop/jazz cadence color** — §2C/§2E leftovers: plagal-sigh melodic formula 6→♭6→5 in the cadence engine; aeolian ♭VI–♭VII–i cadence; jazz ii–V substitution in bars 9–10 of the 12-bar blues schema; turnaround I–vi–ii–V; "cadence-aware final bars" as a third schema variation operator (alongside rotation/substitution).
- [ ] **Pentatonic harmony schema** — §2E: roots drawn from the pentatonic scale, chord quality free; add as a `SchemaKind` or a separate generator mode.
- [ ] **Cross-lane climax separation** — §2A: "two simultaneous melodies must not place climaxes together." Section climax orchestration works within each engine; coordinate across lanes (e.g. vocal vs lead synth) so simultaneous melodies don't peak in the same bar.
- [ ] **EmbellishmentStyle UI wiring** — the `EmbellishmentStyle` knob on `MelodyParams` (Folk/PopBallad/Jazz/Auto, added in 20d3581) is serde-only; nothing in the inspector sets it — same gap the schema generator had. Add a control to the melody/motif inspector. MUST route through the `ux-design` agent first, then `e2e-tester` for snapshot verification.

### Tier 4 polish (small)

- [x] **Huron descending-step bias** — §2A: descending steps slightly more common than ascending; add as a soft weight in `choose_direction` (`motif_engine/build.rs`).
- [ ] **Fix `Augment`/`InvertAugment` washout** — the tiling realizer normalizes duration ratios per chord, so augmentation mostly washes out at render time (noted in fd2581b, pre-existing for plain `Augment`). Make augmentation audible or drop it from the drawn vocabulary.
