# Lyric pronunciation (G2P and dictionary)

> Verified against master @ `cb3b1d6a` + pronunciation persistence (slice 11) on 2026-10-07.
> Spec history: prototype `docs/intents/prototypes/svs-pronunciation-control/index.html` (design #173, notes #174).

Every sung syllable becomes a list of ARPAbet phonemes before it reaches
the voicebank. By default the CMU pronouncing dictionary decides, with a
rule-based fallback. An agent can override how a word is pronounced for
the whole project; the entry is saved with the project and undoes like any
other edit. The data model also has per-syllable overrides (persisted too)
and a global user dictionary, but nothing sets those yet.

## Behaviour

- **Auto path.** `resonance_music_theory::g2p` loads the bundled CMU dict
  (`resonance-music-theory/data/cmudict.dict`, via `cmudict-fast`) and
  falls back to spelling rules for unknown words. It also counts and marks
  syllables (`auto_syllabify_text`), parses `[..]` raw-phoneme blocks and
  slurs, and assigns syllables to notes (`assign_syllables_to_notes_with`).
- **Precedence.** `override > project dictionary > global dictionary > CMU
  auto` (`resolve_clip`). Provenance (`auto` / `EDIT` / `DICT`) is kept per
  syllable for display.
- **Pronunciation source.** Alongside `PhonemeProvenance`, every
  `ResolvedSyllable` and `AssignedSyllable` carries a finer
  `source: g2p::G2pSource` — `Edited` (override or `[..]` block),
  `ProjectDict` (a user dictionary — project **or** global: `resolve_clip`
  merges both into the one `PhonemeDictionary` the resolver sees, so the
  source cannot tell them apart and reports `project_dict` for either),
  `BankDict` (reserved
  for voicebank dictionaries, never produced yet), `Cmu` or `Rules` (the
  spelling fallback). Serde writes it snake_case (`"project_dict"`), and
  `G2pSource::as_str` gives the same string. Slur notes inherit the held
  syllable's source. `g2p::word_g2p_source(word)` answers CMU-or-rules for
  a single word. Nothing in the app reads `source` yet (`song.vocal`'s
  `g2p` field is slice 10 of
  [vocal-mcp-control-and-quality](../../intents/compose/vocal-mcp-control-and-quality.md)).
- **Syllable split.** A word's phonemes are split at the user's `·`
  breaks exactly (see
  [lyrics and melody](vocal-lyrics-and-melody.md#behaviour)); a mismatch
  between pieces and vowels first tries the word's other CMU variants
  (when no `(N)` hint is given), then falls back to the automatic split
  and sets `syllable_warning`.
- **Project dictionary.** `vocal.set_pronunciation` stores a word →
  phonemes entry (word cleaned and lower-cased by `clean_word`; phonemes
  canonicalized, unknown symbols rejected, empty list rejected);
  `vocal.clear_pronunciation` removes it. Entries apply to every occurrence
  of the word in every lane on the next render. They are project-wide, not
  per lane.
- **Voicebank validation.** After resolution, `validate_for_voicebank`
  substitutes phonemes the bank lacks and blocks the render on symbols that
  are not ARPAbet or cannot be substituted (see
  [vocal rendering](vocal-rendering.md)).
- **Persisted.** `PronunciationState` (project dictionary + per-syllable
  overrides) is written to `ProjectFile::pronunciation`
  (`{ dictionary: [{word, phonemes}], overrides: [{definition_id,
  track_id, syllable_index, text, phonemes, variant_idx?}] }`, left out of
  the file when empty; a file without it loads with none) by
  `PronunciationState::to_project`, and restored by the `pronunciation`
  reconcile domain (`restore_from_project`) on project open **and on every
  undo/redo**. Load re-canonicalises phonemes and drops unknown symbols
  (and a dictionary entry left with none). `vocal.set_pronunciation` and
  `vocal.clear_pronunciation` are one undo entry each. Overrides are
  written verbatim, including those of a deleted lane, so undoing the
  delete brings them back.
- **Per-syllable overrides.** Keyed by vocal lane `(definition_id,
  track_id)` and the syllable's index in the lane's resolved draft (`_`
  holds take a note but no syllable index), not by clip and note, so a
  regenerate keeps them and they also apply to a fresh roll. Each stores
  the syllable's text (`ResolvedSyllable::label`); the resolver ignores an
  override whose syllable no longer reads that text (trimmed,
  case-insensitive) or no longer exists. It stays stored, so writing the old
  lyric back revives it. No message, view or control method creates
  overrides yet.
- **Global dictionary.** `DictionaryScope::Global` and the resolver support
  it, but the global dictionary file is never loaded (the resolver is
  passed an empty slice).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| G2P | `resonance-music-theory/src/g2p/{phonemes,syllabify,lyric_parse,note_assign}.rs` | `canonical_phoneme`, `cmu_variants`, `auto_syllabify_text`, `resolve_draft_with_dict`, `assign_syllables_to_notes_with`, `PhonemeProvenance`, `G2pSource`, `word_g2p_source` |
| App model + resolver | `resonance-app/src/compose/vocal_svs/pronunciation.rs` | `PronunciationState` (`lane_overrides`, `resolve_lane`, `to_project`, `restore_from_project`), `DictionaryEntry`, `DictionaryScope`, `SyllableOverride`, `resolve_clip`, `clean_word`, `canonicalize_phonemes` |
| Persisted shape | `resonance-app/src/project/pronunciation.rs` | `ProjectPronunciation`, `ProjectPronunciationEntry`, `ProjectSyllableOverride` |
| Save / restore | `update/project_io/serialize.rs` (`build_project_file`), `update/project_io/reconcile/globals.rs` | `Pronunciation` reconcile domain |
| Control mutation | `resonance-app/src/update/compose/mod.rs` | `control_set_pronunciation`, `control_clear_pronunciation` |
| Control validation | `resonance-app/src/update/control/vocal.rs` | `set_pronunciation` param checks |
| Read view | `resonance-app/src/update/control/song.rs` | `VocalView::pronunciation_overrides` |
| Strip display | `resonance-app/src/view/compose/vocal_roll/` | read-only phoneme strip under the chord strip |

## Surfaces

- **GUI:** read-only phoneme strip in the vocal roll. No editing.
- **Control API / MCP:** `vocal.set_pronunciation` →
  `mcp__resonance__vocal_set_pronunciation` (`word`, `phonemes` lowercase
  ARPAbet); `vocal.clear_pronunciation` → `vocal_clear_pronunciation`;
  `song.vocal` lists `pronunciation_overrides`.
- **Agent skills:** external `singer-songwriter` / `resonance-studio`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Resolver precedence | `resonance-app/tests/vocal/pronunciation_resolution.rs` | `./scripts/run-tests.py -p resonance-app` |
| Resolved phonemes in the segment | `tests/vocal/vocal_segment_pronunciation.rs` | same |
| Control set/clear | `tests/control/control_vocal.rs` | same |
| Save → reopen, undo/redo, override text check, legacy file | `tests/control/control_vocal_pronunciation_persistence.rs` | same |
| G2P | `resonance-music-theory/tests/g2p.rs`, `g2p_apis.rs` | `./scripts/run-tests.py -p resonance-music-theory` |
| `G2pSource`, exact breaks, corpus all-CMU | `resonance-music-theory/tests/g2p_apis/lyric_markers.rs` | same |

Not covered: per-syllable overrides through a real surface (none exists).

## Decisions and deviations

Prototype vs build: the resolver, precedence, provenance, project
dictionary (control only), its persistence and per-unit partial re-render
are built. The interactive phoneme strip (click a syllable, remove/reorder
phonemes, pick a CMU variant, split/merge syllables, revert), the
dictionary editor and the global dictionary with import/export are not.

Overrides were keyed by `(ClipId, note_index)`; slice 11 re-keyed them to
`(definition, track, syllable index)` plus the syllable text, as the
pronunciation-editing intent recommended. "Dropped when the text changes"
is done at resolve time (the override is ignored, not deleted), since
resolving is pure and lyric edits do not walk the override table.

## Known gaps

- No GUI editing, global dictionary never loaded —
  [pronunciation-editing](../../intents/compose/pronunciation-editing.md).
