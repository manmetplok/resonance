# Vocal lanes: lyrics and melody generation

> Verified against master @ `f9438e38` + the `singing` skill, voice presets and the
> chord-change melody flow (BUG-70) on 2026-10-13.
> Spec history: [`docs/history/specs/vocal-mcp-control-and-quality.md`](../../history/specs/vocal-mcp-control-and-quality.md) §3, §6 (slices 1, 3, 8); `docs/history/specs/melody-generation-research.md` (vocal cadence/climax/srdc items); field reports `docs/history/reviews/field-report-2026-10-06.md`.

A vocal lane is a `(section, vocal track)` lane with the Vocal generator.
It holds a lyric **draft** (lines of syllables), melody settings and voice
settings. Generating writes a melody — one note per syllable, plus the
`_` holds the lyrics write and the melismas `SyllableMode` asks for — into the
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
- **Draft markers** (resolved in `resonance-music-theory`):
  - **Melisma `_`** (alias `+`): holds the previous syllable onto one
    more note — `love _ _` is one syllable over three notes. A standalone
    `-` is **not** a hold in the draft (prose writes it as a dash:
    `wait - no` is two syllables); it is punctuation and dropped. Only the
    per-note side table (`is_slur_lyric`) still reads `-` as a slur. A
    hold is not a syllable: `ResolvedSyllable::slurs_after` counts the
    holds after a syllable, `g2p::note_unit_count` gives syllables + holds,
    and `assign_syllables_to_notes_with` turns each hold into a slur note
    (`is_slur`, label `+`, the held vowel, the held syllable's
    `syllable_index`, stress, provenance and source). Holds therefore never
    shift later syllables' indices. A `_` before a line's first sung token
    has nothing to hold and is dropped. A per-note slur annotation on a note
    the draft already holds is the same hold, not an extra one; a per-note
    *label* annotation on such a note ends the remaining holds and takes
    the next syllable. The held syllable's final consonants are sung at the
    end of the hold: they move from the first note to the last slur note,
    and `is_word_end` moves with them (`love _ _` → `l ah` · `ah` ·
    `ah v`; `carry_coda_to_last_hold`). Inside a `·`-broken word a piece
    that is exactly `_` (or `+`) holds the piece before it:
    `a·_·ma·zing` sings `a` over two notes (`split_in_word_holds`); a
    trailing `love·_` equals `love _`, and a leading `_·word` holds the
    previous word like a standalone `_`. The bulk lyric editor keeps the
    `·` next to such a piece (`g2p::strip_syllable_marks`) so the hold
    survives the editor writing the text back.
  - **Breath `AP`** (standalone, uppercase): takes no note and sets
    `breath_before` on the next sung syllable / its note (also across a
    line break; a trailing `AP` at the end of the draft is dropped). The
    renderer sings it as an `AP` breath at the end of the gap before that
    note ([vocal rendering](vocal-rendering.md) → Rests and breaths).
    Lowercase `ap` is a word; `[AP]` is still an inline phoneme block that
    occupies its own note.
  - `count_syllables` (and so the melody generator's per-line note count)
    agrees: it sums `lyric_parse::note_units`, which walks the resolver's
    own tokenizer without dictionary lookups — a word counts its `·` / `-`
    pieces (`re-so-lu-tion` is 4), a `[..]` block its `·` groups
    (`[l ih · l iy]` is 2), a hold one, and a leading hold, an `AP` or a
    punctuation-only token (`&`) none; at least 1 per line. The
    generator's per-note stress table gives holds no stress. `song.vocal`'s
    `syllable_count` is the same note-unit count (`note_unit_count`), so
    `love _ _` on three notes is not a mismatch.
- **Exact user breaks.** A word's `·` pieces are its note labels verbatim,
  and its phonemes split at the matching consonant: each piece gets one
  vowel nucleus, and the cluster between two nuclei is cut so the next
  piece's onset is `max(leading(next), cluster − trailing(prev))`, clamped
  to the cluster. `leading` counts the consonant sounds a piece starts with
  (`th sh ch ng ph wh ck` count as one, `qu` as two, `y` is a consonant
  only piece-initially before a vowel); `trailing` counts the ones it ends
  with by the same digraph rules (plus `gh`, doubled letters as one, `x`
  as two, a final `y` as a vowel, a silent final `e` skipped) —
  `func·tion` → `f ah ng k | sh ax n`, `fun·ction` → `f ah ng | k sh ax n`,
  `ev·ery·thing` → `eh v | r iy | th ih ng`,
  `re·mem·ber` → `… m | b er`, `an·y` → `eh n | iy`, `e·ver` → `eh | v er`.
  Without a `(N)` hint, a word whose pieces do not match the default CMU
  variant's vowel count takes the first variant that does (`ev·ery` →
  `EH V R IY`, `ev·er·y` → `EH V ER IY`). When no variant fits, the
  phonemes fall back to the automatic onset-maximising split (labels stay
  the user's pieces, note count unchanged) and those syllables carry
  `syllable_warning`; no corpus syllable carries one (guarded by a test).
- **Holds: user and generator** (spec §6). A slur note is held either by a
  `_` in the draft (a **user** hold) or by a `"+"` in the clip's per-note
  side table (`compose.vocal_audio.clip_lyrics`) with no draft hold owed —
  a **generator** hold. `g2p::hold_origins` tells them apart with the
  note walk's own rule (a syllable's draft holds go to the first slur
  notes after it, so a run reads user holds first, then generator ones).
  Generator holds are written only by `roll_vocal_melody`
  (`vocal_midi_install::auto_hold_annotations`), so every regenerate
  replaces them; user holds ride in the lyrics and survive `vocal.generate`
  and chord-driven regenerates.
- **The `s` key** (vocal roll, `update/midi_editor/vocal_slur.rs`) edits
  the draft. On a sung note it makes that note hold the syllable before
  it — every hold between that syllable and the note becomes a user hold
  (`g2p::set_holds_after` writes `love _ me` after a word's last
  syllable, `for·_·get` inside a word), and the side-table markers on
  those notes are cleared on every placement's clip of the lane. On a
  generator hold it converts it (and the generator holds before it in the
  run) into user holds the same way. On a user hold it removes one `_`
  from that syllable. A line's first note, a note no syllable precedes,
  and a syllable inside a `[..]` block are no-ops. The edited line is
  locked (a lyric re-roll keeps it), the open bulk editor is refreshed,
  and the press is one undo entry (`ToggleSlur` records a snapshot). The
  notes do not move: until the next generate a new `_` leaves one lyric
  unit more than notes, which `counts_mismatch` shows. A vocal clip that
  belongs to no lane keeps the old per-note `"+"` toggle.
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
  Rise, Fall, Wave, Flat), `SyllableMode` (below), chord-tone anchor, leap range, phrase length, breath, stay-in-scale,
  avoid clashes, use-section-motif. Post-passes: cadence formulas per line,
  one section climax, srdc layout, no overlaps. Needs chords and a
  non-empty draft; otherwise nothing is generated.
- **`SyllableMode`** (`melody/melisma.rs`, `derive_vocal_with_holds`,
  which `roll_vocal_melody` calls). After the syllabic melody is final, a
  seeded pass (its own `XorShift`, so the syllabic passes' draws are
  untouched) splits some notes into 2–3 notes over the note's own time
  span; every note after the first is a generator hold
  (`VocalDerivation::auto_holds`). **Syllabic** runs no pass —
  bit-identical to `derive_vocal_with_motif` (pinned by the existing
  golden). **Mixed**: at most one melisma per line, the line's last unit
  with p = 0.5, otherwise a stressed syllable of at least half a beat with
  p = 0.2; 3 notes with p = 0.25 when the note is long enough.
  **Melismatic**: up to three per line, p = 0.9 / 0.55, 3 notes with
  p = 0.5. A note needs two sixteenth-triplets to split; one followed by
  a draft hold is never split (the walk would hand the hold to the
  ornament). Ornaments: a line-final syllable ends on its cadence pitch
  (upper or lower neighbour → pitch; `pitch · lower · pitch` for three),
  others step toward the next note or turn on a lower neighbour; every
  candidate stays in range, under the line's peak (single-climax rule)
  and within the style's adjacency cap (4 for Hymnal, else 9), else the
  pitch repeats. The pieces sit a 256th apart and keep the syllable's
  velocity. `derive_vocal_with_motif` itself stays one note per unit, so
  `vocal_phrase_spans` (the synth fill) sees the same time spans.
- **Seeds.** GUI generate buttons and `vocal.generate` without a seed bump
  the lane seed; an explicit seed is installed first (reproducible).
- **Default voicebank.** A new lane sings TIGER's `mystic`
  (`VocalParams::default()`: `VoicebankId::DEFAULT` = `tiger`,
  `VocalParams::DEFAULT_SPEAKER` = `mystic`) whatever its voice type — the
  voice that scored best in the 41-voice blind listen of 2026-10-13
  ([vocal follow-ups](../../intents/compose/vocal-followups.md)). Only a
  bank *switch* picks the speaker by voice type (the bank's default). A
  lane keeps the bank and speaker it stored. `vocal.render` without a
  voicebank renders every lane with the bank it holds, so an explicit
  choice is honoured (BUG-52 fixed). The built-in Vocal songwriting
  template sings TIGER Mystic too, in a tenor range inside TIGER's
  comfortable C3–A4.
- **Settings over the wire.** `vocal.get_params` / `vocal.set_params`
  read and patch a lane's settings (everything but the draft) through
  snake_case wire types (`VocalParamsPatch`, `VocalParamsView` in
  `resonance-control/src/methods/vocal.rs`); `VocalParams`' own serde —
  the project format — is untouched. A patch merges into the lane, the
  merged result is validated with `VocalParams::validate()` before any
  change (`invalid_params` naming the field), and the write is one
  `SetLaneGenerator` carrying the lane's seed and draft — one undo entry.
  It never renders or regenerates. `voicebank` is a bank id: any
  installed bank (`vocal.voicebanks` lists them), one of the built-in
  `tiger`, `lilia`, `meiji`, or one the plok.org index lists (see
  [voicebank library](voicebank-library.md)); any other id is
  `invalid_params` (a typo guard), and so is a bank that cannot sing
  English. `speaker` is one of
  that bank's speaker ids, effect modes included (a single-voice bank
  refuses one); a bank switch
  without `speaker` resets it to the bank's default
  (`VocalParams::switch_voicebank` with `BankProfile::default_speaker`,
  which `vocal.render {voicebank}` and the inspector's picker use too). A `theme` over 240 characters (`THEME_MAX_CHARS`, the
  GUI's truncation limit) is refused rather than cut. `vocal.get_params`
  stays answerable while a GUI drag is open. Four render fields live only
  on the lane, outside the generators: `rest_threshold_beats` (0.125–4,
  default 0.5; shortest gap sung as silence), `velocity_dynamics`
  (0–1, default 0.5; velocity → loudness), and `bank_timing` /
  `bank_pitch` (default true; time and pitch the line with the bank's own
  duration and pitch models where it ships them) — see
  [vocal rendering](vocal-rendering.md). All read at their defaults
  from projects written before they existed.
  `section.set_lane_generator` vocal `options` take the same patch and
  merge into an existing lane (lyrics and seed kept; an explicit `seed`
  wins), or into the defaults for a new one.
- **Persistence.** `VocalParams` (draft included) persists inside the lane
  config; per-note lyric annotations of the derived clip persist through
  the `ClipLyrics` side table. The bank is written as its id
  (`"voicebank": "tiger"`, `VoicebankId`, lowercase) and the voice mode as
  `"speaker": "vinyl"` (or `null`). Reading goes through
  `params/legacy.rs` (`#[serde(from)]`): older files' enum names
  (`"Tiger"`, `"Lilia"`, `"Meiji"`) lowercase into the ids, and their
  per-bank `singer` (TIGER, `"Glam"` …) / `singer_meiji` (Meiji,
  `"Standard"` …) become `speaker` from the field that belonged to the
  lane's bank (`disco` / `standard` when absent); they are never written
  back. A missing bank reads as `lilia`, not the current TIGER default:
  every file since phase 2 of the vocal spec stores the field, so only
  older files miss it, and those were last heard on Lilia
  (`params/legacy.rs`, `LEGACY_VOICEBANK`).

### Chord and length changes under a melody (BUG-70)

A chord edit (add / edit / move / resize / delete / replace, a generated
progression) or a section-length change re-derives the section's lanes. A
vocal lane goes through `update/compose/vocal_melody.rs`
(`on_chord_change`) instead of being re-derived blindly:

- **Nothing to lose.** When the lane holds no notes yet, nothing derives
  (no draft, no chords), or the re-derived melody equals what the lane
  holds (notes *and* the per-note lyric table, so a toggled slur counts),
  it is re-derived as before, unasked and unreported.
- **Otherwise** the lane gets one of three outcomes:
  - **Keep** — notes untouched; the lane is flagged
    `melody_predates_chords`. On a length change the kept notes are
    re-installed at the new clip length (notes past a shrunk end cut,
    grown bars silent).
  - **Re-fit** — `resonance_music_theory::refit_melody` moves each pitch to
    the nearest fitting note: a chord tone of the chord sounding at its
    start for a strong-beat note (bar start, middle of an even bar of four
    or more beats, a chord change) or a note a beat or longer; a scale tone
    otherwise (the section's scale, the chord track's key, or — with
    neither — the major / natural-minor key inferred from the chords,
    `infer_scale`; a chromatic scale leaves weak notes alone). **Contour:**
    each note keeps its direction from the previous note — up, down or a
    repeat — measured against the previous note *as re-fitted*: of the
    fitting pitches within a tritone, the nearest one that keeps the
    direction wins, and only if none does the nearest overall (equal
    distances take the lower). So a note that already fits stays put
    unless the previous note's move would flip its direction, and a repeat
    stays a repeat whenever the repeat fits. Count, rhythm, velocity and
    the lyric table never change, and the clip is edited in place (same
    id). Clears `melody_predates_chords` — only when the lane had a clip to
    write to.
  - **Regenerate** — `rederive_vocal_melody`, the old behaviour.
- **Who chooses.** A control call's `vocal_melody`; else the project's
  remembered choice; else, from the GUI, a question; from control, Keep.
- **The GUI question** (`update/compose/vocal_melody_prompt.rs`,
  `view/compose/vocal_melody_prompt.rs`, root overlay
  `Overlay::VocalMelodyPrompt`). The edit dispatches as usual with the
  lanes kept and parked as undecided; after the outermost update (and,
  for a Begin…Commit gesture, after its commit) the modal lists every
  affected lane — section · track, "Notes edited by hand" / "Generated,
  untouched" — with Keep / Re-fit / Regenerate, pre-set to Keep for a
  hand-edited lane and Regenerate for an untouched one. "Apply to all
  lanes" sets every row; "Remember for this project" needs one choice for
  every row. **Apply** carries the choices out and marks the project
  edited without a history entry, so the chord edit's own entry (its
  snapshot predates the edit) takes back both with one undo, and redo
  brings both back. **Cancel**, Esc and the backdrop undo that entry and
  drop the redo step. The question remembers the undo history's
  generation (`UndoHistory::generation`) and closes — lanes kept and
  flagged, a notice says so — if the history moves without it (an undo,
  a control edit, a load); the notice says "undone" when an undo closed
  it and "kept" otherwise. A chord drag publishes one `MoveChord` /
  `ResizeChord` at the release, so it asks once.
- **Gestures and sliders.** While a Begin…Commit gesture is open, a
  chord or length change touches no vocal note: the lane is flagged and
  parked, and gets its outcome (Keep re-lays it) once, at the final
  length, after the commit — then the question opens. The schema
  substitution slider records one coalesced entry per drag: its lanes are
  parked until its release (`VocalMelodyMsg::ControlReleased`, the
  slider's `on_release`); any other edit before that leaves them kept and
  flagged. Opening the question, Apply and Cancel each break the coalesce
  run, and extending a run bumps the history generation, so a later
  nudge is an entry of its own and Cancel never takes back an answered
  one.
- **Remembered choice.** `ProjectFile::vocal_melody_choice`
  (`compose.vocal_melody.remembered`): project state, saved and riding
  undo like the cycle-record mode. Settings › General › Compose "Ask
  again about vocal melodies" clears it (one undoable edit); it is dimmed
  when nothing is remembered and the hint says what is.
- **Flags** (`LaneMelodyFlags` on `LaneGeneratorConfig::melody`, left out
  of the file when clear, so they save and ride undo):
  `hand_edited` is set by every note-editing `MidiEditorMessage` on the
  lane's clip — vocal roll, `notes.*`, `notes.import_midi`, a slur toggle
  — through `vocal_melody::note_hand_edit` (the clip comes from
  `gates::midi_editor_target`, shared with the frozen-input gate), and
  also clears `melody_predates_chords` (edited again since). Any
  (re)generate clears both (`install_vocal_melody`). A vocal config that
  replaces a vocal config with clear flags (`vocal.set_params`,
  `section.set_lane_generator`) keeps the old flags.
- **Badge.** While `melody_predates_chords` is set the lane inspector
  shows a warm "MELODY PREDATES CHORD CHANGE" card with Re-fit and
  Regenerate (`view/compose/vocal_melody_badge.rs`), and the vocal-roll
  toolbar the same chip and buttons. Regenerate here re-derives with the
  lane's seed (no seed bump).
- A kept, re-fitted or regenerated lane goes `stale` like any edit: the
  GUI debounce re-renders, a control client calls `vocal.render`.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Params, lyrics, melody | `resonance-music-theory/src/derive/vocal/{params,params/legacy,lyrics,mod}.rs`, `melody/`, `style/` | `VocalParams`, `VoicebankId`, `VocalParamsRepr`, `generate_lyrics`, `derive_vocal_with_motif`, `vocal_phrase_spans`, `count_syllables` |
| G2P / syllables | `resonance-music-theory/src/g2p/` | `auto_syllabify_text`, `syllabify`, `lyric_parse`, `note_assign`, `holds` (`hold_origins`, `set_holds_after`, `line_first_syllables`, `strip_syllable_marks`) |
| Melisma pass | `resonance-music-theory/src/derive/vocal/melody/melisma.rs` | `derive_vocal_with_holds`, `VocalDerivation` |
| `s` key | `resonance-app/src/update/midi_editor/vocal_slur.rs` | `lane_for_clip`, `toggle_draft_hold` |
| Per-note readout | `resonance-app/src/update/control/view_model/vocal_notes.rs` | `lane_note_lyrics` |
| Lane inspector handlers | `resonance-app/src/update/compose/lane_inspector/vocal_params.rs`, `common.rs` | `generate_all`, `generate_melody_only`, `generate_lyrics_only`, `auto_syllabify_lyrics`, `update_vocal` |
| Lyric buffer | `resonance-app/src/update/compose/vocal_lyrics.rs` | `rebuild_draft_from_bulk`, `sync_bulk_lyrics_from_draft`, `handle_bulk_lyrics_action` |
| Generate orchestration | `resonance-app/src/update/compose/vocal_render.rs` | `roll_vocal_lyrics`, `draft_vocal_lyrics`, `roll_vocal_melody` |
| Lane lookup for control | `resonance-app/src/update/compose/vocal_control.rs` | `first_vocal_definition`, `vocal_definitions_for_track`, `control_set_lyrics`, `control_set_line` |
| Views | `resonance-app/src/view/compose/lane_inspector/vocal/` (`lyrics_group`, `draft_group`, `melody_group`, `voice_group`, `generate_group`), `vocal_lane/`, `vocal_roll/` | right rail, lane row (lyrics + contour), vocal roll editor |
| Control | `resonance-app/src/update/control/vocal.rs`, `song.rs`, `section.rs`, `view_model/vocal_params.rs` | `normalize_lyric_text`, `apply_params_patch`, `vocal_params_view`, `build_vocal_lane_config`, `song.vocal` view model |
| Chord-change outcomes | `resonance-app/src/update/compose/vocal_melody.rs`; model `resonance-app/src/compose/vocal_melody.rs` | `on_chord_change`, `apply_outcome`, `refit_lane`, `note_hand_edit`, `with_requested_choice`, `LaneMelodyFlags`, `VocalMelodyChoice` |
| The question | `resonance-app/src/update/compose/vocal_melody_prompt.rs`, `view/compose/vocal_melody_prompt.rs`, `view/compose/vocal_melody_badge.rs` | `begin_update`, `settle`, `apply`, `cancel`; modal, badge card / chip |
| Re-fit | `resonance-music-theory/src/derive/refit.rs` | `refit_melody`, `infer_scale` |
| Control | `resonance-app/src/update/control/harmony/vocal_melody.rs`, `update/control/vocal/refit.rs` | `run_with_vocal_melody`, `refit_melody` |

## Surfaces

- **GUI:** Compose tab → vocal lane → right rail groups Lyrics, Lyric
  draft, Melody, Voice & delivery, Generate (all / lyrics only / melody
  only, re-render). Double-click opens the vocal roll: notes with their
  syllable, read-only chord strip and phoneme strip (each label is cut
  with `…` to its pill and the next note's start, and left out when not
  even two characters fit), slur arcs, f0 preview;
  edits go through the shared MIDI editor messages. A user hold paints as
  before with a `_` label; a generator hold paints fainter (lower body
  alpha, `WARM_LINE` outline and dashes) with a warm `+`; a note whose
  syllable carries `syllable_warning` gets a `BAD` underline and a corner
  tick (shape plus colour). `s` / `+` is the hold key (above).
- **Control API / MCP:**
  - `vocal.set_lyrics` → `mcp__resonance__vocal_set_lyrics` (`text`, one
    line per `\n`, `syllabify` default true, optional `section_id`).
  - `vocal.set_line` → `vocal_set_line` (`line_index` 0-based within the
    lane, as `song.vocal` lists it).
  - `vocal.get_params` / `vocal.set_params` → `vocal_get_params` /
    `vocal_set_params` (partial snake_case patch; see Behaviour).
  - `vocal.voicebanks` → `vocal_voicebanks` and `vocal.voicebank_install`
    → `vocal_voicebank_install`: the installed and downloadable banks
    ([voicebank library](voicebank-library.md)).
  - `vocal.voice_presets` / `vocal.apply_voice_preset` /
    `vocal.save_voice_preset`: a singer + delivery (bank, speaker, voice,
    range and the delivery fields — never lyrics, generator settings or
    curves) put on a lane in one undo step, or saved from one; the
    inspector's Voice & delivery card has the same picker
    ([voice presets](voice-presets.md)).
  - `vocal.generate` → `vocal_generate` (`seed`, `lyrics` default true —
    pass `false` to keep lyrics you set); returns the derived `clip_id`.
    Also queues a render; when the lane's bank (or its vocoder) is not
    installed the melody is still written and the result carries
    `render_error {kind: voicebank_missing, message}`.
  - `song.vocal` → `song_vocal`: per lane its `params` (the
    `vocal.get_params` shape), its `lines` (indexed within the lane) with
    syllables and phonemes, notes, out-of-range notes, comfortable range
    and voicebank id; per lane its render state (`render_state`,
    `stale_units`, `error {kind, message}`) and the track's (the worst
    lane's) — see [vocal rendering](vocal-rendering.md#render-state).
    Lyric, note, chord, param and pronunciation edits made through the
    control API leave the lane `stale`; only `vocal.render` (or
    `vocal.generate`) renders.
    Per note: `slur`
    (`"user"` — a `_` in the lyrics, `"auto"` — a generator melisma,
    `null` on a sung note), `g2p` (`edited`, `project_dict`, `bank_dict`,
    `cmu`, `rules`; holds inherit the held syllable's) and
    `syllable_warning` (omitted when false), built in
    `update/control/view_model/vocal_notes.rs`. `counts_mismatch`
    compares `syllable_count` with the notes **minus the generator holds**
    (they are not lyric units), so a Mixed / Melismatic lane does not
    flag. Per lane `hand_edited` and `melody_predates_chords`. New
    optional fields: no protocol bump.
  - `vocal.refit_melody` → `vocal_refit_melody` (`track_id`, optional
    `section_id` — the track's first lane): re-fits the lane's pitches to
    its section's current chords; returns `{revision, section_id,
    note_count, notes_changed}`; refuses a lane without notes.
  - `harmony.add_chord` / `edit_chord` / `delete_chord` /
    `apply_progression` and `section.resize` take an optional
    `vocal_melody: keep | refit | regenerate` and report
    `vocal_lanes_affected: [{definition_id, track_id, outcome}]` (edit /
    delete now answer `ChordEditResult`, resize `ResizeResult`: the old
    `{revision}` plus the list).
- **Agent skills:** the agent plugin's `singing` skill is the procedure
  for vocal lanes (voice, lanes, melody, lyrics, pre-flight, render,
  expression); `arranging` points at it. External `singer-songwriter`
  drives the songwriting around it.

## How it's tested

| What | Where | Run |
|---|---|---|
| Control lyrics/lines/lanes | `resonance-app/tests/control/control_vocal.rs`, `control_vocal_lanes.rs`, `control_vocal_generate.rs` | `./scripts/run-tests.py -p resonance-app` |
| Params API, lane-generator merge, per-lane lines, BUG-52 | `resonance-app/tests/control/control_vocal_params.rs` | same |
| Wire shapes | `resonance-control/tests/{methods,views}.rs` | `./scripts/run-tests.py -p resonance-control` |
| Vocal roll | `tests/compose/vocal_roll_keys.rs`, `vocal_roll_key_focus.rs`, `vocal_roll_fingerprint.rs` | same |
| Placeholder rows | `tests/compose/compose_vocal_placeholder.rs` | same |
| Bank id / speaker and legacy migration; a new lane is TIGER Mystic, a file without `voicebank` Lilia | `resonance-music-theory/tests/vocal.rs` (`legacy_enum_bank_and_singer_fields_map_onto_id_and_speaker`, `a_new_lane_sings_tiger_mystic_whatever_its_voice_type`, …), `resonance-app/tests/control/control_vocal_params.rs` (`a_new_lane_starts_on_tiger_mystic`) | `./scripts/run-tests.py -p resonance-music-theory` |
| Theory | `resonance-music-theory/tests/{vocal,vocal_climax,vocal_syncopation,vocal_melody_golden,vocal_params_validate,vocal_enum_labels,g2p,g2p_apis}.rs` | `./scripts/run-tests.py -p resonance-music-theory` |
| Draft markers, exact breaks, corpus all-CMU | `resonance-music-theory/tests/g2p_apis/lyric_markers.rs` (module of `g2p_apis`) | same |
| In-word holds, `set_holds_after`, `hold_origins` | `resonance-music-theory/tests/g2p_apis/holds.rs` | same |
| `SyllableMode`: Syllabic unchanged, Mixed/Melismatic deterministic holds, time spans kept | `resonance-music-theory/tests/vocal_melody_golden/melisma.rs` (module of `vocal_melody_golden`) | same |
| `s` key (insert / remove / in-word / line start / auto → user, undo, survives `vocal.generate`), `slur` / `g2p` / `syllable_warning`, `counts_mismatch` with auto holds | `resonance-app/tests/control/control_vocal_holds.rs` | `./scripts/run-tests.py -p resonance-app` |
| Vocal roll user / auto hold and split-warning golden | `tests/compose/vocal_roll_holds.rs` → `tests/snapshots/compose_vocal_roll_holds-wgpu.png` | `RESONANCE_BLESS=1` |
| Goldens | `tests/snapshots/compose_vocal_*-wgpu.png` (placeholder, wired up, lyrics draft folded) | `RESONANCE_BLESS=1` |
| Chord change over control: hand-written melody kept and flagged, generated lane kept by default, a note edit clears the flag, `regenerate` equals `vocal.generate` with the lane's seed, `refit` keeps rhythm / count / velocity / lyrics and fits the new chords, `vocal.refit_melody` (and its refusal), one `edit.undo` takes back edit + outcome, a remembered choice answers control, resize keeps and re-lays, an unchanged melody reports nothing | `resonance-app/tests/control/control_vocal_melody.rs` | `./scripts/run-tests.py -p resonance-app` |
| The question: asks and keeps until answered, Apply + edit = one undo entry (undo / redo both), Cancel / Esc restores the chords and leaves no redo, remember skips it until "Ask again", hand-edited rows default to Keep, a chord-lane drag asks once at its release, the badge's Re-fit / Regenerate | `resonance-app/tests/compose/compose_vocal_melody_prompt.rs` | same |
| Goldens: the question, the rail badge, the vocal-roll chip | `tests/snapshots/compose_vocal_melody_prompt-wgpu.png`, `compose_vocal_rail_melody_predates-wgpu.png`, `compose_vocal_roll_melody_predates-wgpu.png` | `RESONANCE_BLESS=1` |
| Re-fit theory: invariants, already-fitting notes stay, contour tie-break, repeats, inferred key, a generated melody re-fitted to another progression | `resonance-music-theory/tests/vocal/refit.rs` (module of `vocal`) | `./scripts/run-tests.py -p resonance-music-theory` |

Not covered: theme/POV/clichés (they have no effect).

## Decisions and deviations

- **Params API** (spec §3, D1–D4, D8). One `vocal.get_params` /
  `vocal.set_params` pair with partial patches; a `set_lane_generator`
  re-issue merges into the existing lane and keeps lyrics and seed; wire
  enums are snake_case and `range` is `{low, high}`; `song.vocal` carries
  each lane's full params and its own `lines[]`, indexed the way
  `vocal.set_line` indexes them. The wire types are separate from
  `VocalParams`' serde, so the project format did not change; the change
  of enum names, `song.vocal` shape and string bank ids took
  `PROTOCOL_VERSION` to 2, with the agent plugin's lockstep re-read.
  The patch never carries `draft` or the expression curves (those are
  `automation.*`, see [automation control API](../timeline/automation-control-api.md)).
- **Default bank.** Slice 1 (D9) made Lilia the default; the 41-voice
  blind listen of 2026-10-13 moved it to TIGER `mystic` (scored 5; TIGER's
  mean 4.00 against Lilia's 3). It also takes the default off Lilia's ToU,
  which bans generative-AI use. TIGER's terms are `paid` (see the
  [voicebank library](voicebank-library.md)). The `vocal.render` Lilia
  remap went with slice 1 (BUG-52).
- Lyrics are template-based (no language model); the corpus is small, so
  re-rolls repeat lines quickly.
- `lane_generator.rs` still says vocal generation is "stubbed in
  `derive_vocal`"; it is not — that comment is stale.
- A chord edit never renders a vocal lane: whatever its outcome, the lane
  goes `stale`; a GUI edit arms the debounced re-render, a control edit
  waits for `vocal.render`.
- **The chord edit is not staged** (BUG-70 design). It dispatches with
  the affected lanes kept, the safe outcome; the question's Apply folds
  its outcome into the edit's undo entry instead of recording one, and
  Cancel is an undo of that entry plus dropping its redo step. Staging the
  edit until the answer would have left the history entry the edit's
  `Record` classification already pushed empty.
- **Defaults and remember.** Keep for hand-edited rows, Regenerate for
  untouched ones; over control the default is always Keep (never lose
  notes silently). "Remember" stores one choice, so it needs every row on
  the same choice (the toggle is disabled otherwise). The remembered
  choice is project state and rides undo: undoing the edit whose answer
  set it forgets it again.
- **What "affected" means.** A lane is asked about only when a
  re-derive would change its notes or lyric table — so a resize of an
  untouched lane, whose chords did not change, re-derives silently, while
  a resize under a hand-written melody asks (and Keep re-lays the notes
  at the new length). A resize is reported with the same flag name,
  `melody_predates_chords`.
- **A behaviour change without a protocol bump.** `harmony.*` and
  `section.resize` used to regenerate every vocal melody they reached;
  called without `vocal_melody` they now keep it (and flag the lane). No
  field changed meaning and none was removed — `vocal_melody` and
  `vocal_lanes_affected` are additions — so `PROTOCOL_VERSION` stays; the
  default outcome changed, which the control-API conventions leave to the
  docs, and there are no external clients yet. The agent plugin (0.6.0)
  says so in `singing`.
- **Lanes without a clip mapping.** A lane loaded from a project saved
  before `ProjectFile::derived_clips` resolves its clips by position for
  reading *and* writing (`lane_placement_clips`), and a resize re-lays it
  too.
- **Control surfaces.** `vocal_melody` is on `harmony.*` and
  `section.resize`. `arrangement.insert_bars` / `remove_bars` never
  re-derive a section, so they take none. A time-signature edit that
  trims a section's chords (`transport.set_time_signature`,
  `global.*_signature_event`) re-derives too; it takes no `vocal_melody`
  and follows the default (remembered, else Keep) — see Known gaps.
- `vocal.refit_melody` with no `section_id` re-fits the track's first
  lane, like the other lane-addressed `vocal.*` writes (not every lane,
  as `vocal.render` does).
- Generator holds live in the existing per-note side table as `"+"` —
  no new marker. A `"+"` left by the old per-note `s` toggle in an older
  project therefore reads as a generator hold (no external users).
- Holds inside a word (`a·_·ma`) are an addition to the spec's standalone
  `_`: without them `s` could not hold a word's first syllable.
- A hold run is user holds first, then generator holds, so `s` on a
  generator hold converts every generator hold before it in the run too,
  and removing a user hold shortens the run's user part from its end.
- `SyllableMode` is a post-pass that splits finished notes rather than a
  change to the per-style walks: it keeps Syllabic bit-identical, leaves
  every seeded pass's RNG stream alone and keeps each syllable's time
  span.

- Corpus fixes: `fore·get` → `for·get`; `ev·ry` (not a CMU word) →
  `an·y` and `ev·ery·thing`; `word·s` → `words` and `ev·ery` →
  `ev·er·y` (the Reflective line became `and ev·er·y still·ness makes it
  clear` to keep its declared 9). Each changed line's note count now
  equals its declared `syllables`, and no corpus syllable carries
  `syllable_warning`.

## Known gaps

- Signature edits that trim chords take no `vocal_melody` and do not
  list `vocal_lanes_affected`; their lanes are kept (or get the
  remembered choice).
- A lane that sings at several placements is judged and re-fitted from
  its first placement's clip; a hand edit made to another placement's clip
  only is not compared.

- Breaths are rendered (slice 12) but not drawn in the vocal roll.
- An `s` edit is a lyric edit: the lane goes `stale` and, if it already
  has audio, the debounced GUI re-render brings it up to date 750 ms
  later (see [vocal rendering § render state](vocal-rendering.md#render-state)).
- The bulk lyric editor still drops ordinary `·` breaks when it writes
  text back (only the ones next to an in-word hold survive).

- Theme, POV and "avoid clichés" are ignored —
  [lyric-theme-generation](../../intents/compose/lyric-theme-generation.md).
- No harmony or doubled voices from a lead —
  [vocal-harmony-doubling](../../intents/compose/vocal-harmony-doubling.md).
- Vocal and lead-synth climaxes can land in the same bar —
  [cross-lane-climax-separation](../../intents/compose/cross-lane-climax-separation.md).
