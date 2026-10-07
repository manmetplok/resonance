# Lyric pronunciation (G2P and dictionary)

> Verified against master @ `cb3b1d6a` + voicebank registry (vocal slices 4–5) on 2026-10-07.
> Spec history: prototype `docs/intents/prototypes/svs-pronunciation-control/index.html` (design #173, notes #174).

Every sung syllable becomes a list of ARPAbet phonemes before it reaches
the voicebank. By default the voicebank's own English dictionary decides
for the words it lists, then the CMU pronouncing dictionary, with a
rule-based fallback. An agent can override how a word is pronounced for
the whole project. The data model also has per-syllable overrides and a
global user dictionary, but nothing sets those yet.

## Behaviour

- **Auto path.** `resonance_music_theory::g2p` loads the bundled CMU dict
  (`resonance-music-theory/data/cmudict.dict`, via `cmudict-fast`) and
  falls back to spelling rules for unknown words. It also counts and marks
  syllables (`auto_syllabify_text`), parses `[..]` raw-phoneme blocks and
  slurs, and assigns syllables to notes (`assign_syllables_to_notes_with`).
- **Precedence.** `override > project dictionary > global dictionary >
  bank dictionary > CMU > rules` (`resolve_clip_with_bank`; `resolve_clip`
  is the same without a bank layer). Provenance (`auto` / `EDIT` / `DICT`)
  is kept per syllable for display; a bank-dictionary hit is `auto`.
- **Bank dictionary** (spec §4.4). An installed bank's `dsdict-en.yaml`
  (next to its acoustic config, at its root, or one folder below either —
  `dsdur/` usually) is read on first use (`InstalledBank::dictionary`) into
  a `PhonemeDictionary` and passed to the resolver as
  `g2p::DictionaryLayers::bank`. An entry is kept only if its spelling is
  English letters and every phoneme, with the bank's `en/` prefix
  stripped, is exactly an ARPAbet phone and not silence; the first entry
  for a spelling wins. Lilia's multi-language extras (`dx`, `L`, `d0`)
  therefore fall through to CMU. The entry is also skipped for a word with
  a `(N)` CMU-variant hint, or whose `·` pieces do not match its vowel
  count (CMU's variants may fit the breaks). The vocal roll's phoneme
  strip and `song.vocal` resolve through the same layer.
- **Pronunciation source.** Alongside `PhonemeProvenance`, every
  `ResolvedSyllable` and `AssignedSyllable` carries a finer
  `source: g2p::G2pSource` — `Edited` (override or `[..]` block),
  `ProjectDict` (a user dictionary — project **or** global: `resolve_clip`
  merges both into the one `PhonemeDictionary` the resolver sees, so the
  source cannot tell them apart and reports `project_dict` for either),
  `BankDict` (the voicebank's dictionary, above), `Cmu` or `Rules` (the
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
- **Mapping onto the bank.** After resolution, `validate_for_voicebank`
  maps every phone onto the bank's inventory (`VoicebankPhonemes`): kept
  if the bank has it bare or `en/`-prefixed; otherwise the bank's
  substitution hint (Lilia `v` → `f`, `b`), then the voiced/voiceless
  partner, then the nearest phone of the same articulatory class
  (`nearest_by_class`: vowels to vowels, stops to stops, …, never across
  the vowel/consonant line). A symbol that is not ARPAbet, or that nothing
  near enough reaches, blocks the render; the error names the note, the
  syllable, the **word** (`InvalidSyllable::word`) and the bank. The
  language prefix is added when the segment is built
  (`BankProfile::phoneme_name`) — see [vocal rendering](vocal-rendering.md).
- **Not persisted.** `PronunciationState` (project dictionary + per-clip
  overrides) is runtime-only. It is not written to the project file, and
  `ComposeState::load_from_project` clears it — which runs on project open
  **and on every undo/redo** (the `ComposeSections` reconcile domain). So a
  dictionary entry is lost on reload and on any undo.
- **Per-syllable overrides and global dictionary.** The types
  (`SyllableOverride`, `DictionaryScope::Global`) and the resolver support
  them, but no message, view or control method creates them, and the
  global dictionary file is never loaded (the resolver is passed an empty
  slice).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| G2P | `resonance-music-theory/src/g2p/{phonemes,syllabify,lyric_parse,note_assign}.rs` | `canonical_phoneme`, `cmu_variants`, `auto_syllabify_text`, `resolve_draft_with_dict`, `resolve_draft_with_layers`, `DictionaryLayers`, `assign_syllables_to_notes_with`, `PhonemeProvenance`, `G2pSource`, `word_g2p_source` |
| App model + resolver | `resonance-app/src/compose/vocal_svs/pronunciation.rs` | `PronunciationState`, `DictionaryEntry`, `DictionaryScope`, `SyllableOverride`, `resolve_clip`, `resolve_clip_with_bank`, `clean_word`, `canonicalize_phonemes` |
| Bank dictionary + inventory mapping | `compose/vocal_svs/registry/dict.rs`, `phonemes.rs`, `validate.rs`; `resonance-svs/src/voicebank/dsdict.rs` | `load_english_dictionary`, `VoicebankPhonemes`, `nearest_by_class`, `validate_for_voicebank`, `load_dsdict` |
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
| G2P | `resonance-music-theory/tests/g2p.rs`, `g2p_apis.rs` | `./scripts/run-tests.py -p resonance-music-theory` |
| `G2pSource`, exact breaks, corpus all-CMU | `resonance-music-theory/tests/g2p_apis/lyric_markers.rs` | same |
| Bank-dictionary layer precedence | `resonance-music-theory/tests/g2p_apis/bank_dictionary.rs` | same |
| Bank dictionary from a fixture bank, prefix stripping, inventory mapping, unmappable word | `resonance-app/tests/vocal/bank_phonemisation.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: survival across save/load or undo (it does not survive).

## Decisions and deviations

Prototype vs build: the resolver, precedence, provenance, project
dictionary (control only) and per-unit partial re-render are built. The
interactive phoneme strip (click a syllable, remove/reorder phonemes, pick
a CMU variant, split/merge syllables, revert), the dictionary editor, the
global dictionary with import/export, and persistence are not.

## Known gaps

- No GUI editing, no persistence, cleared on undo, global dictionary never
  loaded — [pronunciation-editing](../../intents/compose/pronunciation-editing.md).
