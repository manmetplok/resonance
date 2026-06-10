# Improving melody & harmony generation — research notes

Date: 2026-06-10. Sources: code audit of `resonance-music-theory` + Open Music
Theory v2 (https://viva.pressbooks.pub/openmusictheory/, chapter URLs cited
inline). Scope: synth/instrument melodies, vocal melodies, harmony generation.

## 1. Why current output is boring and samey (diagnosis)

The generators produce *correct* music — in range, in scale, consonant with the
chords — but optimize for safety, not interest. Concrete causes, with code
locations:

1. **No phrase grammar.** Phrases are realized independently by tiling the
   motif over chords (`derive/motif_engine/phrase.rs`, `realize_phrase`).
   There is no presentation → continuation structure, no fragmentation, no
   acceleration toward a goal. Every phrase is the same blueprint.
2. **No real cadences.** Instrument melodies never target a cadence formula.
   Vocal cadences are deterministic (even lines always land on 2/4/7, odd
   lines on 1/3/5 — `derive/vocal/style/mod.rs:233-299`), so endings are
   simultaneously predictable *and* theory-weak (no 2→1 / 7→1 over V→I).
3. **No single climax.** Contours are fixed math curves applied per phrase
   (`phrase.rs:155-171` arch/asc/desc/wave; vocal `derive/vocal/melody.rs:32-42`).
   A wave contour has two peaks; consecutive phrases each get their own peak.
   Nothing enforces one section-level climax.
4. **Over-constrained pitch choice.** Strong beats snap to chord tones, weak
   beats snap to scale (`align_to_harmony`), and `apply_gap_fill` auto-fills
   any leap > 5 semitones. There is no embellishing-tone vocabulary
   (appoggiatura, suspension, escape tone, anticipation) — i.e. no controlled
   dissonance, hence no tension/relaxation.
5. **Uniform rhythm.** Motifs pick one of 8 fixed duration patterns
   (`motif_engine/build.rs:15-24`) and tile it; vocals are grid-aligned with
   ±0.08 jitter. No syncopation transform, no tresillo, no rhythmic
   acceleration toward cadences.
6. **Weak variation operators.** Transforms (`phrase.rs:92-151`) are single
   ops (transpose/invert/retrograde/augment/diminish/fragment) drawn
   independently per phrase. No sequences (transposed repetition by a
   consistent interval), no combined ops, no antecedent/consequent pairing
   ("same opening, different ending").
7. **Harmony is locally plausible, globally shapeless.** The Markov sampler
   (`generator/markov.rs`, order ≤ 2) can't model a phrase-level T→PD→D→T
   arc, and the builtin tables (`generator/table.rs`) contain no schema
   identity — output wanders between adjacent-plausible chords instead of
   looping a recognizable progression.

OMT frames the goal directly: a good melody balances tension and relaxation —
"music that simply makes it easy for the brain to parse is boring"
(chapter/species-counterpoint/). Our generators only do the "easy to parse"
half.

## 2. Codable principles from Open Music Theory

### A. Well-formed line rules (validator / cost function)
(chapter/first-species-counterpoint/, chapter/second-species-counterpoint/)
- Range of a phrase ≤ a 10th, usually < octave.
- Mostly steps; leaps occasional, mostly 3rds. Forbid melodic tritones, 7ths,
  augmented/diminished intervals, > octave; avoid 6ths.
- **Leap recovery**: leap ≥ 4th ⇒ next motion is a step in the opposite
  direction. (We currently do the opposite — `apply_gap_fill` erases the leap
  retroactively instead of resolving it.)
- ≤ 2 leaps in a row; consecutive same-direction leaps only if they outline
  one triad. Also check the interval *between direction-change extrema* —
  don't outline a tritone/7th.
- **Exactly one climax** per phrase, not the final note, ideally approached by
  leap, in the second half (melodic arch). Secondary lower climaxes OK in
  long lines. Two simultaneous melodies must not place climaxes together.
- End by step: 2→1 (usual) or 7→1; leading tone resolves up.
- Huron's statistical tendencies as soft weights: pitch proximity; descending
  steps slightly more common; **step inertia** (continue direction more often
  than not — our 50/50 coin violates this); **melodic regression** (after a
  register extreme, head back toward the middle).

### B. Phrase grammar: sentence / period / srdc
(chapter/phrase-archetypes-unique-forms/, chapter/melody-and-phrasing/)
- **Sentence** (8 bars): *presentation* = basic idea (2 bars) + varied repeat,
  tonic-prolonging harmony, no cadence; *continuation* = fragmentation
  (2-bar ideas → 1-bar fragments), faster surface rhythm, faster harmonic
  rhythm, optional sequence, drives to a cadence.
- **Period**: antecedent ends weak (HC, sometimes IAC); consequent starts with
  the *same basic idea* and ends strong (PAC). Weak→strong is the defining
  feature — a repeated phrase with identical endings is not a period.
- **Pop srdc** (statement, restatement, departure, conclusion): aaba / aabc,
  one lyric line ≈ one 4-bar phrase, 2–4 phrases per section. Departure =
  fragmentation + move away from tonic; conclusion returns or concludes.

### C. Cadence formulas
(chapter/intro-to-harmony/, chapter/strengthening-endings-with-v7/,
chapter/cadential-64/, chapter/strong-predominants/)
- PAC: melody 2→1 or 7→1 over root-position V→I. IAC: melody ends 3 or 5.
  HC: end on V, melody 1→7 or 3→2. Strength order HC < IAC < PAC.
- Pre-dominant: bass 4→5 via ii6 (most common) or IV; IV precedes ii when both.
- Cadential 6/4: over bass 5, resolve 6→5 and 4→3 in the same voices.
- Tendency tones: 7→1, 4→3, 2→1. Deceptive: V→vi.
- Pop cadences: plagal IV–I; plagal sigh IV–iv–I with melody 6→♭6→5; aeolian
  ♭VI–♭VII–i(/I); mixolydian ♭VII as dominant substitute.
- Jazz: ii–V–I (m7–7–maj7 / ø7–7–m7), turnaround I–vi–ii–V.

### D. Embellishing-tone vocabulary
(chapter/embellishing-tones/) — generator table (approach/leave/beat):

| Tone | Approach | Leave | Beat |
|---|---|---|---|
| Passing | step | step, same dir | weak (fills a 3rd) |
| Neighbor | step | step, opposite | weak |
| Appoggiatura | leap | step, opposite (usually down) | **strong** |
| Escape | step | leap, opposite | weak |
| Suspension | held | step **down** | **strong** |
| Retardation | held | step **up** | **strong** |
| Anticipation | — | becomes next chord tone | weak |

Dissonance discipline: never leap both into *and* out of a dissonance; this is
what makes added non-chord tones sound intentional rather than wrong.

### E. Pop harmonic schemas (loopable banks)
(chapter/4-chord-schemas/, chapter/blues-based-schemas/,
chapter/modal-schemas/, chapter/puff-schemas/, chapter/pentatonic-harmony/)
- 12-bar blues (+ jazz ii–V substitution in bars 9–10).
- Doo-wop I–vi–IV–V (variant I–vi–ii–V).
- Axis / singer-songwriter I–V–vi–IV ≡ vi–IV–I–V — *any rotation*.
- Hopscotch IV–V–vi–I (post-2010); minor reading VI–VII–i–III.
- Plagal vamp I–IV; double plagal ♭VII–IV–I; extended plagal ♭III–♭VII–IV–I.
- Lament i–♭VII–♭VI–V. Circle of fifths vi–ii–V–I.
- Puff I–iii–IV as a phrase *opener* (not a loop).
- Modal shuttles: I–♭VII, i–♭VII–♭VI–♭VII (aeolian), i–IV (dorian), I–II♯
  (lydian).
- Pentatonic harmony: roots from the pentatonic scale, quality free.
- Variation operators on schemas: **rotation** (start elsewhere in the loop)
  and **function-preserving substitution** (swap chords sharing ≥ 2 tones).

### F. Rhythm
(chapter/rhythm-and-meter-in-pop-music/, chapter/hypermeter/)
- **Straight syncopation transform**: halve the first of N equal durations,
  shift every later note earlier by that half. One-line transform, apply at
  8th or 16th level. The canonical pop rhythm device.
- **Tresillo** 3+3+2 (and 3+2+3, 2+3+3); double tresillo 3+3+3+3+2+2.
- **Hypermeter**: 4-bar groups; cadences and phrase starts on hyper-downbeats;
  harmonic rhythm accelerates approaching the cadence.

### G. Voice leading (for harmony rendering / future SATB-ish voicing)
(chapter/chords-in-satb-style/) — bass first, melody second (tendency-tone
correct, built backwards from the cadence), inner voices nearest chord tones;
no parallel 5ths/octaves; never double the leading tone or a chordal 7th;
prefer contrary motion against a rising 4→5 bass.

## 3. Recommendations, prioritized

### Tier 1 — highest impact on "boring and samey"

1. **Phrase grammar in the motif engine.** Replace per-phrase independent
   tiling with sentence/period/srdc plans: phrase 1 = basic idea + varied
   repeat; phrase 2 = fragmentation of the idea's head + denser rhythm +
   cadence target. Periods: consequent reuses the antecedent's opening
   transform but swaps the ending. Touches `plan_motif_transforms` and
   `realize_phrase` (`derive/motif_engine/phrase.rs`).
2. **Cadence targeting.** Give every phrase a goal: pick HC/IAC/PAC (weak for
   antecedent, strong for consequent), and force the last 2 melody notes to a
   formula (2→1, 7→1, 3→2, 1→7) compatible with the underlying chord. Replace
   the fixed vocal degree sets in `cadence_pitch` with the same formula table
   + occasional deceptive choice.
3. **Single-climax enforcement + leap recovery.** Post-process (or generate
   with) the well-formed-line rules: one highest note per phrase in its
   second half, approached by leap; leap ≥ 4th followed by opposite step
   (replace `apply_gap_fill`'s leap-erasure); step inertia instead of the
   50/50 direction coin; melodic regression at register extremes.
4. **Embellishing-tone pass.** Generate a chord-tone skeleton on strong beats,
   then decorate via the table in §2D with style-weighted probabilities
   (pop ballad: suspensions + appoggiaturas; folk: passing/neighbor;
   jazz: anticipations + escape tones). This is the principled replacement
   for "snap everything to chord/scale".
5. **Rhythm transforms.** Keep the 8 base patterns but add: straight
   syncopation (probability per phrase), tresillo cells, and rhythmic
   acceleration in continuations (halve note values when fragmenting).
   Vocals: allow division-level syncopation on stressed syllables instead of
   pure grid + jitter.

### Tier 2 — harmony

6. **Schema bank alongside the Markov sampler.** Add `GeneratorSpec::Schema`
   drawing from §2E (axis, doo-wop, hopscotch, lament, plagal family, modal
   shuttles, blues, circle of fifths), with rotation + function-preserving
   substitution + cadence-aware final bars as variation. Markov stays for
   "wander" mode; schemas give sections an identity listeners recognize.
7. **Phrase-model overlay on Markov output.** Constrain sampled progressions
   to T→PD→D→T per phrase (function tags per degree per table), accelerate
   harmonic rhythm into bar 4/8, and place cadential dominants on
   hyper-downbeats.

### Tier 3 — structure & variation depth

8. **Section-level contour orchestration.** One climax per *section*, placed
   in a designated phrase (e.g. phrase 3 of 4); other phrases get lower
   secondary peaks. Replaces independent per-phrase contour draws; also fixes
   vocal lines where every line independently arches.
9. **Sequences as a transform.** Add real sequences (model + transposed
   copies: descending fifths, descending thirds, ascending 5–6) to the
   transform vocabulary, used in continuations/departures.
10. **Composable transforms.** Allow transform pairs (e.g. fragment +
    transpose, invert + augment) at high complexity; widen the operator set
    rather than the randomness.

### Parameter-level quick wins (no architecture change)

- Step direction: 50/50 → ~0.62 continue (step inertia), with regression bias
  near register edges.
- `repeat_chance` 0.11 is fine, but cap *consecutive* repeats at 2.
- Forbid generated leaps of 6 semitones (tritone) and > 7; currently leaps are
  uniform 3–7.
- Vocal antecedent/consequent: keep the role concept but randomize *which*
  formula within the role, and add ~10% deceptive endings.

## 4. Suggested implementation order

1. Quick wins + leap recovery + single climax (small, contained in
   `motif_engine/build.rs` / `phrase.rs`, immediately audible).
2. Cadence formula targeting (melody + vocal), then phrase grammar
   (sentence/period planner) — these two reinforce each other.
3. Embellishing-tone decoration pass.
4. Rhythm transforms (syncopation/tresillo/acceleration).
5. Harmony schema bank + phrase-model overlay.
6. Section-level climax orchestration, sequences, composable transforms.

Each step is independently testable: the well-formed-line rules in §2A double
as assertions for unit tests (range, leap recovery, single climax, ending by
step), which gives the generators a regression net we currently don't have.
