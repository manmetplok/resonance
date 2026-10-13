# Compose

The Compose area is where a song's structure and generated material live:
section definitions placed on the timeline, a chord grid and scale per
section (plus a song-wide chord track for pinned harmony and the song key),
per-lane generators that turn chords into bass, lead, pad and vocal MIDI,
the drum pattern bank and per-section drum arrangement, the vocal pipeline
from lyrics to sung audio through DiffSinger, and the pure
`resonance-music-theory` crate under all of it. Code lives in
`resonance-app/src/compose/`, `update/compose/`, `view/compose/`,
`chord_track.rs`, `chord_sheet_pdf.rs`, `chord_box.rs`,
`resonance-music-theory/` and `resonance-svs/`. Open work is in
[`docs/intents/compose/`](../../intents/compose/).

| Feature | Doc | One-line summary |
|---|---|---|
| Sections and placements | [sections-and-placements.md](sections-and-placements.md) | Create, resize, place and delete section definitions; placements never overlap; meter comes from the tempo map. |
| Lane generators and derived clips | [lane-generators.md](lane-generators.md) | Bass/Melody/Pad generators per (section, track) write derived MIDI clips on every placement; `generate.part`, `section.set_lane_generator`. |
| Section chords and progressions | [section-chords-and-progressions.md](section-chords-and-progressions.md) | Per-section chord grid, scale, Markov style-table and schema progression generators; `harmony.*`, `section.set_scale`. |
| Chord track and song key | [chord-track.md](chord-track.md) | Song-wide chord regions whose pins constrain regeneration, and the song key set by `transport.set_key`. |
| Drum patterns and section arrangement | [drum-arrangement.md](drum-arrangement.md) | Project pattern bank of polymetric drum groups, per-section arrangement with repeats and fills, built-in grooves; `generate.drums`. |
| Vocal lyrics and melody | [vocal-lyrics-and-melody.md](vocal-lyrics-and-melody.md) | Vocal lanes: lyric draft, template lyric generator, style-based vocal melody; `vocal.set_lyrics`, `vocal.set_line`, `vocal.generate`, `song.vocal`. |
| Vocal rendering | [vocal-rendering.md](vocal-rendering.md) | DiffSinger synthesis via `resonance-svs`, cached partial re-renders, rests and breaths, velocity dynamics, expression curves, per-lane render state; `vocal.render`. |
| Voicebank library | [voicebank-library.md](voicebank-library.md) | Installed DiffSinger banks (user folder, dev roots), the plok.org index and installs with their vocoders, the library overlay; `vocal.voicebanks`, `vocal.voicebank_install`. |
| Voice presets | [voice-presets.md](voice-presets.md) | A singer + delivery put on a vocal lane in one step: 31 built-ins from the blind listen (best first, scored), user presets as JSON; `vocal.voice_presets`, `vocal.apply_voice_preset`, `vocal.save_voice_preset`. |
| Lyric pronunciation | [pronunciation.md](pronunciation.md) | CMU G2P with a project pronunciation dictionary and voicebank phoneme substitution; `vocal.set_pronunciation`. |
| Music-theory library | [music-theory-library.md](music-theory-library.md) | `resonance-music-theory`: pitch, scale, chord, voicing, SATB, progression generators, G2P, fretboard. |
| Melody and motif engine | [melody-and-motif-engine.md](melody-and-motif-engine.md) | Motif-based melody with phrase forms, sequences, cadence formulas, single climax, embellishing tones and rhythm transforms. |
| Chord sheet PDF export | [chord-sheet-export.md](chord-sheet-export.md) | Printable chord diagrams for guitar, bass and keyboard from the section chords. |

Related areas: the timeline draws sections and the chord lane and moves
placements on bar insert/remove ([timeline](../timeline/)); the control
API plumbing is in [control-api](../control-api/); vocal pitch-correction
of recorded audio is engine-side ([audio-engine](../audio-engine/)), with
its editor still an intent
([vocal-pitch-editor](../../intents/compose/vocal-pitch-editor.md)).
