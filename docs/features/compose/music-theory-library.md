# Music-theory library (`resonance-music-theory`)

> Verified against master @ `acac5ea3` + chord spelling (BUG-72) on 2026-10-15.
> Spec history: `docs/history/specs/melody-generation-research.md`, `docs/history/specs/todo.md`.

`resonance-music-theory` is the pure, headless crate behind everything
Compose generates: pitch and scale maths, chord parsing and voicing,
progression generators, the part generators (bass, pad, melody, vocal),
lyric G2P and fretboard shapes. It has no audio, app or plugin
dependencies, so all of it is testable without the DAW. This doc maps the
crate; the melody side is in
[melody and motif engine](melody-and-motif-engine.md).

## Behaviour

- **Pitch / scale.** `PitchClass` (12, with `midi_note_name`), `Scale {
  root, mode }`, `Mode` = Chromatic, Major, Minor, Dorian, Phrygian,
  Lydian, Mixolydian, Locrian, HarmonicMinor, MelodicMinor.
- **Chords.** `Chord { root, quality, bass }` with 15 `ChordQuality`
  values; `parse_chord` accepts symbols like `Am7`, `F#m7b5`-style names,
  slash basses and unicode accidentals; `Chord::inverted`.
- **Spelling.** `PitchClass` stores no accidental; `PitchClass::name(Spelling)`
  spells it with sharps or flats. `Scale::prefers_flats` (major parent F,
  Bb, Eb, Ab, Db or Gb), `Scale::spelling`, `Scale::root_name`;
  `Chord::display_in(Option<&Scale>)` / `Chord::spelled(Spelling)` give a
  `SpelledChord` whose text parses back to the same chord. Plain
  `Display` for `Chord` and `PitchClass` stays on sharps; `Scale`'s
  `Display` spells its own root (`Bb major`).
- **Diatonic harmony.** `diatonic_chord(scale, degree, seventh)`,
  `diatonic_triads`, `degree_function` (`Function` T/PD/D),
  `walk_progression` (the older progression walker, kept for
  `progression_seed`).
- **Progression generators** (`generator/`): `GeneratorSpec` with
  `MarkovProgression` (tables in `TableRegistry::with_builtins`: pop,
  modal, jazz, post_rock, metal, classical; order-n back-off; start/end
  degree; locks), `Schema` (`SchemaKind`, rotation, substitution) and
  `Pentatonic` (`PentatonicFlavour`). Output `GeneratedMaterial` carries
  `GeneratedChord { degree, locked }`, `SplitChord`s from the phrase-model
  overlay (`generator/markov/{phrase,cadence,fill}.rs`) and inversions /
  cadential 6/4 (`generator/inversion.rs`, `Degree::is_cadential_six_four`).
- **Voicing.** `close_voicing`, `voice_lead`, `nearest_midi_*`; SATB
  voice leading `satb_voicings` (bass first, no parallel 5ths/8ves, no
  doubled leading tone or chordal 7th) used by `derive_pad`.
- **Part generators** (`derive/`): `derive_bass`, `derive_bass_motif`,
  `derive_pad`, `derive_melody`, `derive_motif_melody_with_section`,
  `derive_melody_fill_vocal`, `derive_vocal*`, `generate_lyrics`,
  `MotifSource` / `MotifParams` / manual motif cells.
- **G2P** (`g2p/`): CMU dictionary (`data/cmudict.dict`, licence in
  `data/LICENSE-CMUDICT`), ARPAbet inventory, syllabification, lyric
  parsing, note assignment, phoneme dictionaries — see
  [pronunciation](pronunciation.md).
- **Fretboard** (`fretboard.rs`): `Tuning` (`GUITAR_6`, `GUITAR_8`,
  `BASS_4`, `BASS_5`), `voicing` / `voicing_from` in a four-fret window.
- **Determinism.** All randomness goes through the crate-private
  `rng::XorShift` seeded by callers, so the same seed and inputs give the
  same output; golden tests pin that.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Primitives | `resonance-music-theory/src/{pitch,scale,chord,voicing,progression}.rs` | `PitchClass`, `Scale`, `Mode`, `Chord`, `parse_chord`, `diatonic_chord` |
| Progression generators | `src/generator/{mod,table,schema,pentatonic,degree,inversion}.rs`, `generator/markov/` | `GeneratorSpec`, `Generator::generate`, `TableRegistry`, `SchemaKind`, `HarmonicFunction` |
| SATB | `src/satb.rs` | `satb_voicings`, `chordal_seventh` |
| Part generators | `src/derive/` | see [melody and motif engine](melody-and-motif-engine.md) |
| G2P | `src/g2p/` | `canonical_phoneme`, `auto_syllabify_text`, `resolve_draft_with_dict` |
| Fretboard | `src/fretboard.rs` | `voicing`, `Tuning`, `ALL_TUNINGS` |

Consumers: `resonance-app` (Compose, chord sheet, control parsing),
`resonance-svs` (G2P in its example CLI), and `resonance-audio` narrowly
(`Mode`/`Scale` for vocal-tuning scale snap, per ARCHITECTURE.md).
Dependencies: `cmudict-fast`, `phf`, `serde`, `strum`, `thiserror`.

## Surfaces

- **GUI:** indirectly, through every Compose generator, the chord lane and
  the chord sheet.
- **Control API / MCP:** indirectly — `harmony.*` parse symbols with
  `parse_chord` and render numerals with `diatonic_chord`;
  `section.set_lane_generator` / `generate.part` deserialize
  `BassParams` / `MelodyParams` / `PadParams` / `VocalParams` straight
  from `options`, so every serde field of those structs is reachable over
  the wire.
- **Agent skills:** none directly.

## How it's tested

All in `resonance-music-theory/tests/`, run with
`./scripts/run-tests.py -p resonance-music-theory`:

| Topic | Files |
|---|---|
| Primitives | `pitch.rs`, `scale.rs`, `scale_snap.rs`, `chord.rs`, `chord_parse.rs`, `chromatic_degrees.rs`, `voicing.rs`, `fretboard.rs`, `tooltip_claims.rs` |
| Progressions | `progression.rs`, `generator.rs`, `markov_golden.rs` (golden data inline), `schema_generator.rs`, `pentatonic_generator.rs`, `phrase_overlay.rs`, `inversions.rs`, `satb.rs` |
| Bass / pad / derive basics | `derive_basics.rs`, `bass_motif.rs` |
| Motif engine | `motif_line_rules.rs`, `motif_descending_bias.rs`, `leap_recovery.rs`, `phrase_climax.rs`, `section_climax.rs`, `phrase_forms.rs`, `phrase_mask.rs`, `phrase_sequences.rs`, `composed_transforms.rs`, `rhythm_transforms.rs`, `motif_rhythm.rs`, `embellishment.rs`, `cadence_formulas.rs`, `manual_motif.rs`, `manual_motif_toggle.rs` |
| Vocal | `vocal.rs`, `vocal_climax.rs`, `vocal_syncopation.rs`, `vocal_melody_golden.rs`, `vocal_params_validate.rs`, `vocal_enum_labels.rs` |
| G2P | `g2p.rs`, `g2p_apis.rs` |

Not covered: musical quality is judged by ear; tests pin rules (ranges,
leap limits, cadence formulas, single climax) and seeded goldens, not
taste.

## Decisions and deviations

- The crate stays dependency-free of audio/app code by design
  (ARCHITECTURE.md layering rule, checked by `tools/arch-invariants`).
- Lyrics G2P lives here rather than in `resonance-svs` so the vocal roll
  and the render share one phoneme source.

## Known gaps

The five open items from `docs/history/specs/todo.md`:

- [harmony-melody-phrase-plan](../../intents/compose/harmony-melody-phrase-plan.md)
- [pop-jazz-cadence-color](../../intents/compose/pop-jazz-cadence-color.md)
- [cross-lane-climax-separation](../../intents/compose/cross-lane-climax-separation.md)
- [embellishment-style-ui](../../intents/compose/embellishment-style-ui.md)
- [motif-augment-washout](../../intents/compose/motif-augment-washout.md)
