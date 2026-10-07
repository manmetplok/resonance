# Vocal lanes: lyrics and melody generation

> Verified against master @ `86e5c2af` + vocal-params work on 2026-10-07.
> Spec history: `docs/history/specs/melody-generation-research.md` (vocal cadence/climax/srdc items); field reports `docs/history/reviews/field-report-2026-10-06.md`.

A vocal lane is a `(section, vocal track)` lane with the Vocal generator.
It holds a lyric **draft** (lines of syllables), melody settings and voice
settings. Generating writes a melody — one note per syllable — into the
lane's derived MIDI clip and starts a singing render
([vocal rendering](vocal-rendering.md)). Lyrics can be typed, edited per
line, re-rolled from a small template corpus, or set by an agent.

## Behaviour

- **One lane per (section, track).** A vocal lane only exists after the
  Vocal generator is installed on it (GUI pick list or
  `section.set_lane_generator kind=vocal`). A track that sings in four
  sections needs one lane per section on the same track; control calls that omit
  `section_id` hit the track's first vocal lane in placement order
  (`first_vocal_definition`), except `vocal.render` (see rendering).
- **Lyric draft.** `VocalParams::draft: Vec<LyricLine>` — each line has a
  number, rhyme letter, syllable count, text with `·` syllable breaks, and a
  `locked` flag. Breaks may be typed as `-` or `·`; `[..]` blocks are raw
  phonemes. Auto-syllabify (`g2p::auto_syllabify_text`) inserts the missing
  breaks so each syllable gets its own note.
- **Draft markers** (resolved in `resonance-music-theory`; the app does not
  render them yet beyond what is noted):
  - **Melisma `_`** (aliases `+`, standalone `-`): holds the previous
    syllable onto one more note — `love _ _` is one syllable over three
    notes. It is not a syllable: `ResolvedSyllable::slurs_after` counts the
    holds after a syllable, `g2p::note_unit_count` gives syllables + holds,
    and `assign_syllables_to_notes_with` turns each hold into a slur note
    (`is_slur`, label `+`, the held vowel, the held syllable's
    `syllable_index`, stress, provenance and source). Holds therefore never
    shift later syllables' indices. A `_` before a line's first sung token
    has nothing to hold and is dropped. A per-note slur annotation on a note
    the draft already holds is the same hold, not an extra one. The final
    consonant is still sung on the first note (the spec's "consonant at the
    end of the last note" is not built).
  - **Breath `AP`** (standalone, uppercase): takes no note and sets
    `breath_before` on the next sung syllable / its note (also across a
    line break; a trailing `AP` at the end of the draft is dropped). The
    app never sings it as a word but does not yet render a breath.
    Lowercase `ap` is a word; `[AP]` is still an inline phoneme block that
    occupies its own note.
  - `count_syllables` (and so the melody generator's per-line note count)
    agrees: a hold counts one note, a leading hold and an `AP` count none.
    The generator's per-note stress table gives holds no stress.
- **Exact user breaks.** A word's `·` pieces are its note labels verbatim,
  and its phonemes split at the matching consonant: each piece gets one
  vowel nucleus, and the next piece's onset takes one consonant phoneme per
  leading consonant letter (`th sh ch ng ph wh ck` count as one, `qu` as
  two, `y` is a consonant only piece-initially before a vowel), clamped to
  the cluster — `func·tion` → `f ah ng k | sh ax n`, `fun·ction` →
  `f ah ng | k sh ax n`. When the piece count differs from the vowel count
  the phonemes fall back to the automatic onset-maximising split (labels
  stay the user's pieces, note count unchanged) and those syllables carry
  `syllable_warning`.
- **Editing.** Bulk text editor for the whole lane (`vocal_bulk_lyrics`,
  synced both ways with the draft), per-line text, lock per line,
  "re-roll unlocked", auto-syllabify. Editing lyrics does not change the
  notes; generate the melody again to re-fit it.
- **Lyric generator.** `generate_lyrics` samples a bundled corpus of 56
  pre-syllabified template lines filtered by `VocalMood` (Yearning,
  Defiant, Hopeful, Reflective, Joyful, Melancholy) and arranged by
  `VocalRhymeScheme` (AABB, ABAB, ABCB, ABBA, Free) within the syllable
  range; locked lines stay and anchor their rhyme. Every corpus word is in
  the CMU dictionary (`lyric_corpus_lines`, guarded by a test). The theme
  text (≤240 chars), point of view and "avoid clichés" are stored and
  shown but the generator ignores them.
- **Melody generator.** `derive_vocal_with_motif` walks the section's
  chords once per syllable. Inputs: `VoiceType` (Soprano … Bass, sets the
  default range), range, `VocalStyle` (PopBallad, Conversational, Hymnal,
  Folk, Anthemic, Chant — each its own profile), `VocalContour` (Arch,
  Rise, Fall, Wave, Flat), `SyllableMode` (Syllabic, Mixed, Melismatic),
  chord-tone anchor, leap range, phrase length, breath, stay-in-scale,
  avoid clashes, use-section-motif. Post-passes: cadence formulas per line,
  one section climax, srdc layout, no overlaps. Needs chords and a
  non-empty draft; otherwise nothing is generated.
- **Seeds.** GUI generate buttons and `vocal.generate` without a seed bump
  the lane seed; an explicit seed is installed first (reproducible).
- **Default voicebank.** A lane's `VocalParams::voicebank` defaults to
  Lilia (`default_voicebank`; spec D9 — Lilia's licence terms are accepted
  while Resonance is not commercial). `vocal.render` without a voicebank
  renders every lane with the bank it holds, so an explicitly chosen TIGER
  is honoured (BUG-52 fixed).
- **Settings over the wire.** `vocal.get_params` / `vocal.set_params`
  read and patch a lane's settings (everything but the draft) through
  snake_case wire types (`VocalParamsPatch`, `VocalParamsView` in
  `resonance-control/src/methods/vocal.rs`); `VocalParams`' own serde —
  the project format — is untouched. A patch merges into the lane, the
  merged result is validated with `VocalParams::validate()` before any
  change (`invalid_params` naming the field), and the write is one
  `SetLaneGenerator` carrying the lane's seed and draft — one undo entry.
  It never renders or regenerates. `voicebank` is an id string (`tiger`,
  `lilia`, `meiji`) and `speaker` one string that maps to `singer`
  (TIGER), `singer_meiji` (Meiji) or nothing (Lilia, which refuses one);
  a bank switch without `speaker` resets it to the bank's default.
  `section.set_lane_generator` vocal `options` take the same patch and
  merge into an existing lane (lyrics and seed kept; an explicit `seed`
  wins), or into the defaults for a new one.
- **Persistence.** `VocalParams` (draft included) persists inside the lane
  config; per-note lyric annotations of the derived clip persist through
  the `ClipLyrics` side table.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Params, lyrics, melody | `resonance-music-theory/src/derive/vocal/{params,lyrics,mod}.rs`, `melody/`, `style/` | `VocalParams`, `generate_lyrics`, `derive_vocal_with_motif`, `vocal_phrase_spans`, `count_syllables` |
| G2P / syllables | `resonance-music-theory/src/g2p/` | `auto_syllabify_text`, `syllabify`, `lyric_parse`, `note_assign` |
| Lane inspector handlers | `resonance-app/src/update/compose/lane_inspector/vocal_params.rs`, `common.rs` | `generate_all`, `generate_melody_only`, `generate_lyrics_only`, `auto_syllabify_lyrics`, `update_vocal` |
| Lyric buffer | `resonance-app/src/update/compose/vocal_lyrics.rs` | `rebuild_draft_from_bulk`, `sync_bulk_lyrics_from_draft`, `handle_bulk_lyrics_action` |
| Generate orchestration | `resonance-app/src/update/compose/vocal_render.rs` | `roll_vocal_lyrics`, `draft_vocal_lyrics`, `roll_vocal_melody` |
| Lane lookup for control | `resonance-app/src/update/compose/vocal_control.rs` | `first_vocal_definition`, `vocal_definitions_for_track`, `control_set_lyrics`, `control_set_line` |
| Views | `resonance-app/src/view/compose/lane_inspector/vocal/` (`lyrics_group`, `draft_group`, `melody_group`, `voice_group`, `generate_group`), `vocal_lane/`, `vocal_roll/` | right rail, lane row (lyrics + contour), vocal roll editor |
| Control | `resonance-app/src/update/control/vocal.rs`, `song.rs`, `section.rs`, `view_model/vocal_params.rs` | `normalize_lyric_text`, `apply_params_patch`, `vocal_params_view`, `build_vocal_lane_config`, `song.vocal` view model |

## Surfaces

- **GUI:** Compose tab → vocal lane → right rail groups Lyrics, Lyric
  draft, Melody, Voice & delivery, Generate (all / lyrics only / melody
  only, re-render). Double-click opens the vocal roll: notes with their
  syllable, read-only chord strip and phoneme strip, slur arcs, f0 preview;
  edits go through the shared MIDI editor messages.
- **Control API / MCP:**
  - `vocal.set_lyrics` → `mcp__resonance__vocal_set_lyrics` (`text`, one
    line per `\n`, `syllabify` default true, optional `section_id`).
  - `vocal.set_line` → `vocal_set_line` (`line_index` 0-based within the
    lane, as `song.vocal` lists it).
  - `vocal.get_params` / `vocal.set_params` → `vocal_get_params` /
    `vocal_set_params` (partial snake_case patch; see Behaviour).
  - `vocal.generate` → `vocal_generate` (`seed`, `lyrics` default true —
    pass `false` to keep lyrics you set); returns the derived `clip_id`.
    Also queues a render.
  - `song.vocal` → `song_vocal`: per lane its `params` (the
    `vocal.get_params` shape), its `lines` (indexed within the lane) with
    syllables and phonemes, notes, out-of-range notes, comfortable range
    and voicebank id; the track's render state.
- **Agent skills:** `arranging` (vocal_generate); external skill
  `singer-songwriter` drives the full lyric flow.

## How it's tested

| What | Where | Run |
|---|---|---|
| Control lyrics/lines/lanes | `resonance-app/tests/control/control_vocal.rs`, `control_vocal_lanes.rs`, `control_vocal_generate.rs` | `./scripts/run-tests.py -p resonance-app` |
| Params API, lane-generator merge, per-lane lines, BUG-52 | `resonance-app/tests/control/control_vocal_params.rs` | same |
| Wire shapes | `resonance-control/tests/{methods,views}.rs` | `./scripts/run-tests.py -p resonance-control` |
| Vocal roll | `tests/compose/vocal_roll_keys.rs`, `vocal_roll_key_focus.rs`, `vocal_roll_fingerprint.rs` | same |
| Placeholder rows | `tests/compose/compose_vocal_placeholder.rs` | same |
| Theory | `resonance-music-theory/tests/{vocal,vocal_climax,vocal_syncopation,vocal_melody_golden,vocal_params_validate,vocal_enum_labels,g2p,g2p_apis}.rs` | `./scripts/run-tests.py -p resonance-music-theory` |
| Draft markers, exact breaks, corpus all-CMU | `resonance-music-theory/tests/g2p_apis/lyric_markers.rs` (module of `g2p_apis`) | same |
| Goldens | `tests/snapshots/compose_vocal_*-wgpu.png` (placeholder, wired up, lyrics draft folded) | `RESONANCE_BLESS=1` |

Not covered: theme/POV/clichés (they have no effect).

## Decisions and deviations

- Lyrics are template-based (no language model); the corpus is small, so
  re-rolls repeat lines quickly.
- `lane_generator.rs` still says vocal generation is "stubbed in
  `derive_vocal`"; it is not — that comment is stale.
- Vocal lanes regenerate MIDI on chord edits but do not re-render audio
  then; audio is re-rendered on demand.

- Corpus fixes: `fore·get` → `for·get`; `ev·ry` (not a CMU word) →
  `an·y` and `ev·ery·thing`. Two corpus words still flag
  `syllable_warning` because their pieces do not match CMU's vowel count:
  `ev·ery` (CMU variant 1 has three vowels) and `word·s`; fixing them
  would change line note counts, so they were left.

## Known gaps

- `song.vocal`, the GUI and the counts check do not yet show `_` holds,
  breaths, `syllable_warning` or the `g2p` source, and breaths are not
  rendered — [vocal-mcp-control-and-quality](../../intents/compose/vocal-mcp-control-and-quality.md)
  slices 8, 10 and 12.

- Theme, POV and "avoid clichés" are ignored —
  [lyric-theme-generation](../../intents/compose/lyric-theme-generation.md).
- No harmony or doubled voices from a lead —
  [vocal-harmony-doubling](../../intents/compose/vocal-harmony-doubling.md).
- Vocal and lead-synth climaxes can land in the same bar —
  [cross-lane-climax-separation](../../intents/compose/cross-lane-climax-separation.md).
