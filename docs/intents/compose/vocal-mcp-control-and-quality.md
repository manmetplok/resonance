# Vocals: MCP control, voicebank library, correctness and expression

> Status: **partly built** (slices 1–5, 9 and 11 built; progress per slice in §12)
> Area: compose (vocals) + control-api + plugins-instruments (library)
> Source: MCP field report 2026-10-07; design Q&A with Jorrit 2026-10-07
> (every decision below was put to him as a multiple-choice question; §11
> lists the answers); research report
> [singing-voicebank-options-2026](../../history/research/singing-voicebank-options-2026.md).
> Absorbs: the former `vocal-expression-curves.md` intent (deleted), slice 1
> of [pronunciation editing](pronunciation-editing.md), BUG-50 and BUG-52.

## 1. Why

An agent can set lyrics and pick a voicebank, and nothing else that shapes
the voice. Over MCP it cannot change the voice type, timbre, vibrato,
tension, portamento, articulation, speaker or melody/lyric generator
settings. It cannot see which voicebanks exist, cannot hold a syllable over
several notes, and gets wrong answers from `song.vocal` about whether a lane
needs re-rendering. Underneath, the render has limits that cap quality even
from the GUI:
- staccato and short rests are impossible;
- there are no written breaths;
- velocity is inaudible on TIGER;
- expression curves are not saved and cannot be reached.

The research added two findings:
- **Bank support is overstated.** No shipped bank's acoustic model takes
  energy or breathiness (every `dsconfig.yaml` has
  `use_energy_embed: false` and `use_breathiness_embed: false`), yet
  `curve_supported` claims Dynamics works everywhere and Breathiness works
  on Lilia and Meiji.
- **The bank set is frozen.** It is three banks fixed in an enum, while
  better-fitting English banks exist (Peiton, the LUNAI English singers,
  Meiji v170).

## 2. Outcome

When all slices have landed:

- An agent reads and sets every lane setting with one partial-update call.
  It sees what each lane will sing and where the pronunciation came from,
  whether the lane is rendered, stale or failed, and which banks are
  installed or downloadable. It can install a bank and render, all without
  the GUI.
- Any DiffSinger bank in OpenUtau layout that can sing English can be
  installed, either from the plok.org index (the files themselves come from
  the creator's own URL) or by dropping a folder in.
- Melisma, rests, breaths, velocity dynamics and expression curves are
  audible, saved and undoable, and reachable from both the GUI and MCP.

## 3. Lane parameters over the wire

### 3.1 Methods

- **`vocal.get_params {track_id, section_id?}`** returns the lane's full
  params, the `seed` and the `revision`. A lane is addressed the same way
  as by the other `vocal.*` methods (`update/control/vocal.rs:680-710`).
- **`vocal.set_params {track_id, section_id?, params: {…partial}}`**
  - **Patch:** every field is optional. Fields that are present merge into
    the lane's current params, and the rest are kept.
  - **Validation:** the merged result goes through `VocalParams::validate()`
    *at set time*. Today validation only runs at render time; a failure now
    becomes `invalid_params` naming the field.
  - **Bank switch:** changing `voicebank` resets `speaker` to that bank's
    default unless the same call also sets `speaker`.
  - **Undo and return:** one undo entry per call; the reply carries the full
    params.
  - **No auto-render:** the call marks the lane stale (§7) and never
    renders on its own. It does not regenerate either; `vocal.generate`
    does that.
  - **Excluded fields:** the patch never carries `draft` (lyrics: use
    `vocal.set_lyrics` / `vocal.set_line`) or the expression curves (§10).
- **`section.set_lane_generator`** with `kind: vocal` merges:
  - on an existing lane, the options are the same partial patch and the
    lyrics and seed are kept;
  - on a new lane, missing fields take their defaults.

  The "must be COMPLETE" text leaves the tool description.
- **`song.vocal`:** each lane gains `params` (the `get_params` shape).
  `lines[]` moves into each lane, indexed the way `vocal.set_line` indexes
  it. Today it is one list across all lanes with a global index
  (`update/control/song.rs:316-333`).

### 3.2 Wire shape

The wire types are new and live in `resonance-control/src/methods/vocal.rs`:
`VocalParamsPatch` (every field `Option`, `deny_unknown_fields`) and
`VocalParamsView`. Enums are **snake_case** (`pop_ballad`, `first_singular`)
and `range` is `{low, high}` in MIDI notes. The app converts between the
wire types and `resonance_music_theory::VocalParams`, so the project format
is untouched. `section.set_lane_generator`'s vocal options switch to the
same snake_case wire type.

The patch groups its fields as follows. Ranges and defaults are in
`params.rs:358-521`; the last two render fields are new in this spec.

| Group | Fields |
|---|---|
| Voice / render | `voicebank` (string id, §4), `speaker` (string, §4), `timbre`, `vibrato`, `vibrato_rate`, `tension`, `tension_velocity_amount`, `tension_contour_amount`, `portamento_ms`, `articulation`, `consonant_emphasis`, **`rest_threshold_beats`** (§8), **`velocity_dynamics`** (§9) |
| Melody generator | `voice`, `range`, `style`, `contour`, `syllable_mode`, `chord_tone_anchor`, `leap_range`, `phrase_length_bars`, `breath`, `stay_in_scale`, `avoid_clashes`, `use_section_motif` |
| Lyric generator | `theme`, `mood`, `pov`, `rhyme`, `lines`, `syllables_min`, `syllables_max`, `match_syllables_to_melody`, `avoid_cliches` |

`singer` (TIGER) and `singer_meiji` disappear into `speaker`.

### 3.3 Protocol

This changes the wire in three ways: the enum names, the `song.vocal`
shape, and the voicebank becoming a string. So **`PROTOCOL_VERSION` goes to
2** (`resonance-control/src/lib.rs:79`), `resonance-agent-plugin/lockstep.json`
follows, and the skills that use vocals are re-read: `arranging` and its
`references/generators.md`, `spatial` and `mixing`.

## 4. Voicebanks as installable content

### 4.1 Identity

- `VocalParams.voicebank` becomes a string id and `speaker` an
  `Option<String>`.
- The ids of the three current banks are `tiger`, `lilia` and `meiji`.
- Loading an old project accepts the old enum names (`Tiger`, …) and maps
  `singer`/`singer_meiji` onto `speaker` (custom deserialise). There are no
  external users, so this compatibility is for our own projects and
  templates only.

### 4.2 Where banks live

- **User banks:** `$XDG_DATA_HOME/resonance/voicebanks/<id>/`, overridable
  with `RESONANCE_VOICEBANK_DIR`. Vocoders go in a sibling
  `…/resonance/vocoders/<id>/`.
- **Development banks:** the existing `RESONANCE_SVS_MODELS_DIR` and
  `resonance-svs/models/` are scanned as well, with fixed ids for the three
  layouts there.
- **What counts as a bank:** a directory holding a `dsconfig.yaml` at most
  two levels deep. That covers OpenUtau layout, including Meiji's nested
  `configs/configs/`.
- **Registry:** a new `VoicebankRegistry` in `resonance-app/src/compose/vocal_svs/`,
  built on the unused `resonance-svs` `VoicebankManifest::scan()`. It is
  rescanned at startup, after an install, and on `vocal.voicebanks`.
- `paths.rs::locate_voicebank` becomes a lookup in the registry.

### 4.3 Facts per bank: derive, then built-in overrides

| Fact | Derived from | Built-in table (keyed by id) |
|---|---|---|
| Speakers | the acoustic `dsconfig` `speakers` list | display names, and the allow-list for known banks. TIGER's acoustic model lists about 36 speakers, most from other creators' banks; only its 7 own modes are offered. |
| Languages / English | `languages.json`, the dictionary files, `en/`-prefixed phonemes | — |
| Phoneme inventory and gaps | the bank dictionary against ARPAbet | substitution hints (Lilia `v`→`f`/`b`, which moves out of `phonemes.rs:127-138`) |
| Curve support | `use_{energy,breathiness,voicing,tension}_embed` on the acoustic config | — (this replaces the hardcoded `paths.rs:218` `curve_supported`) |
| Comfortable range | — | per bank and speaker (moves out of `validate.rs:183-193`). An unknown bank gets 48–76 marked `range_source: "default"`. |
| Credit line, terms URL, commercial-use summary | — | from the index entry (§5) or the table; an unknown bank gets none |
| Vocoder | `dsconfig` `vocoder` field | — |

Banks with no English phonemes are listed with `english: false`, and
`vocal.set_params` rejects them, because lyrics are English.

### 4.4 Phonemisation for any bank

The lookup order is:
1. a user edit;
2. the project dictionary;
3. the bank's own English `dsdict*.yaml` (as OpenUtau does);
4. CMU;
5. spelling rules.

CMU and rules output is mapped onto the bank's inventory:
- add or strip language prefixes;
- apply the bank's substitution hints;
- otherwise substitute the nearest phoneme by articulatory class.

A word that still cannot be mapped is a validation error naming the word.
Meiji's language id (`paths.rs:146-150`) generalises to "the bank's id for
`en`" from `languages.json`.

### 4.5 `vocal.voicebanks`

The call returns `{installed: [...], available: [...]}`:
- **Installed entries:** `id`, `name`, `version`, `english`, `speakers[]`,
  `comfortable_range` (per speaker where known), `missing_phonemes`,
  `substitutions`, `curves` (which expression curves work), `credit`,
  `terms_url`, `commercial`, `vocoder` and `vocoder_installed`.
- **Available entries:** the plok.org index entries that are not installed,
  with their terms.

The call is read-only, sits above the mutation gate and bumps no revision.

### 4.6 Missing bank

`vocal.render`, and the render step of `vocal.generate`, fail with a new
error kind **`voicebank_missing`**. The error names the bank id, and the
vocoder if that is what is missing, and points at `vocal_voicebank_install`
or the library. It replaces the generic "no SVS voicebank is installed"
(`update/compose/mod.rs:614-640`). An empty `ph_seq` gets its own error
instead of sharing that one.

`vocal.set_params` accepts a bank id that is installed or listed in the
index, and rejects any other id as `invalid_params` (typo guard).

## 5. plok.org index and install

- **Index:** `https://resonance.plok.org/index.json` gains `voicebanks[]`
  and `vocoders[]`. The entries are `{id, name, version, languages,
  speakers, size, bytes?, sha256?, url, terms_url, credit, commercial:
  "free_with_credit" | "paid" | "permission", vocoder?, added}`.
- **Upstream files only:** `url` is **the creator's own download URL**
  (a GitHub release, etc.). Bank terms forbid rehosting (TIGER, LUNAI) or
  ask for permission (Lilia), so plok.org holds the index only, never the
  files.
  - `sha256` pins the upstream file. A mismatch fails with "upstream file
    changed; the index needs updating".
  - Maintaining the index is server work outside the repo.
- **Shared download code:** the download machinery in
  `plugins/resonance-drums/src/download.rs` moves to `resonance-common`
  (`plok_download`): index cache, resume, free-space check, timeouts,
  sha256 and the zip install through `drumkit_library/install.rs`. Drums
  and vocals then share it, and the drums keep their behaviour and tests.
- **Install:** unpack into the user voicebank directory and handle a
  wrapper directory. If the bank's vocoder is missing, fetch that index
  entry too. A bank that ships its own vocoder uses it.
- **GUI:** a voicebank library overlay opens from the lane inspector's
  voicebank picker, with tabs `Installed | plok.org`.
  - It reuses `BrowserModel` / `parse_search` and the shared marks under a
    new `voicebank:` kind.
  - Download shows the terms and credit line and needs an explicit accept.
    It also has progress, Cancel, Update and Delete (with confirmation).
- **MCP: `vocal.voicebank_install {id}`** is a job (`job_wait` /
  `job_status`) that installs the bank and its vocoder.
  - Running it counts as accepting the terms; the result carries `terms_url`,
    `credit` and `commercial` so the agent can tell the user.
  - It creates no undo entry and bumps no revision.
- **Index content at first:** TIGER, Lilia and Meiji v170, plus the
  candidates that pass the audition (§12, slice 6). Shipping v170 is how
  Meiji gets upgraded.

## 6. Melisma

- **Lyric token:** `_` as a standalone token holds the previous syllable
  onto the next note. For example, `love _ _` sings "love" across three
  notes, with the final consonant at the end of the last one. `+` is
  accepted as an alias.
  - `tokenize_line` (`g2p/lyric_parse.rs`) stops dropping these tokens and
    emits slur tokens.
  - The melody generator already counts `_` as a note
    (`melody/syllables.rs:9`), so the generator, G2P and the counts now
    agree.
- **Counts:** `counts_mismatch` compares notes with syllables, where a
  syllable is a sung token **or** a `_`. Notes the generator itself made
  into holds (below) are subtracted from the note count.
- **The `s` key** (`update/midi_editor.rs:596-626`) now edits the lyric
  draft: it inserts or removes the `_` at that note's position. The
  per-note `clip_lyrics` slur entries become derived from the draft, so
  `vocal.generate` and regenerates keep user holds, because the lyrics
  carry them.
- **`SyllableMode` is implemented.**
  - **Syllabic:** as today, one note per syllable.
  - **Mixed:** the generator gives some phrase-final and long stressed
    syllables 2–3 notes.
  - **Melismatic:** it does that more often.
  - **Storage:** generator-made holds are per-note slurs that are
    regenerated each time and never written into the draft.
  - **Readout:** `song.vocal` notes show `slur: "user" | "auto"`, and
    pressing `s` on an auto hold converts it to a user `_`.

## 7. Render state per lane

- **States:** each `song.vocal` lane reports
  `render_state: not_rendered | rendering | rendered | stale | error`, plus
  `stale_units` and `error` (kind and message). The top-level
  `render_state` becomes the worst lane state. `Stale` and `Error` exist in
  the enum today (`song.rs:475-486`) but are never produced.
- **How staleness is found:** build the lane's render plan and compare its
  unit `content_key`s (`render_cache.rs:163`) with the units of the audio
  that is installed.
  - All equal means `rendered`; any difference means `stale`.
  - The plan is memoised per lane revision.
  - The installed audio's keys are **saved with the vocal audio**, so a
    reopened project reads `rendered`, not `stale`. Today the cache lives
    only in memory.
- **When re-renders happen:**
  - **GUI:** a lyric, note, chord, param, pronunciation or curve edit
    triggers a debounced re-render of only the changed units, 750 ms
    after the last edit.
  - **MCP:** edits never auto-render, so an agent batches its edits and
    calls `vocal.render` once.
  - `vocal.generate` keeps its documented auto-render.
- **Stale audio** keeps playing until the new render replaces it. The lane
  inspector and the vocal-roll header show a stale badge.
- **Chord edits:** `propagate_chord_change` → `regenerate_lane` →
  `roll_vocal_melody` has its render Task dropped (`regenerate.rs:97-126`).
  Reading the code suggests the lane is left without audio and stuck at
  `rendering`. The Task is passed up so that chord edits follow the rule
  above.

## 8. Syllables, pronunciation sources, rests and breaths

- **User breaks are exact.** `fore·get` sets the note labels and the
  phoneme split:
  - each piece gets one vowel nucleus;
  - the consonant cluster between nuclei is split so that the next piece's
    onset gets one consonant phoneme for each of its leading consonant
    letters (digraphs `th sh ch ng ph wh ck` count as one), clamped to the
    cluster;
  - if the pieces don't match the vowel count, the split falls back to
    automatic and the note shows `syllable_warning`.

  Today `tokenize_line` keeps only the count and `resolve_draft_with_dict`
  re-splits by onset maximisation.
- **Pronunciation source:** every sung note in `song.vocal` carries
  `g2p: "edited" | "project_dict" | "bank_dict" | "cmu" | "rules"`. This
  splits the current `PhonemeProvenance::Auto`, which hides whether CMU or
  the spelling rules were used (detectable via `cmu_variants`).
- **Corpus typo:** `derive/vocal/lyrics.rs:63` `fore·get` becomes `for·get`.
  A new test asserts every corpus word resolves through CMU, never rules.
- **Rests:** the lane param `rest_threshold_beats` (range 0.125–4, default
  **0.5**) replaces `SILENCE_GAP_SEC`:
  - a gap between notes longer than the threshold, converted at the lane's
    tempo, is silence;
  - a shorter gap is sung legato;
  - all three users of `SILENCE_GAP_SEC` change together (`segment/duration.rs:152`,
    `render_cache.rs:112` and `validate.rs:228`).

  At 120 bpm the default is 0.25 s, more detached than today's 0.4 s, so
  vocal goldens are re-blessed. This is an approved sound change.
- **Breath token:** a standalone uppercase `AP` between words.
  - It is a breath before the next word, takes no note, and is not counted
    as a syllable.
  - It fills the end of the gap before the next note, up to 0.35 s. If
    there is no gap, it takes up to 30% (at most 0.25 s) of the previous
    note's tail.
  - Lowercase `ap` stays a word. The bracketed phoneme block `[AP]`, which
    occupies its own note, keeps its existing meaning.
  - **To check first:** leading and trailing pads and rest gaps are emitted
    as `AP` today (`segment/duration.rs:119,152,294`). If that already
    sounds like breathing, plain rests switch to `SP` so that only written
    or generated breaths breathe.

## 9. Velocity dynamics

The lane param `velocity_dynamics` (0–1, default 0.5) maps velocity to
loudness in two parts:
- **Bank curve:** where the bank supports it, velocity drives energy. If the
  bank has no energy curve, it drives tension, which is today's path
  (`segment/tension.rs`).
- **Gain on every bank:** a per-note gain after the vocoder, ±6 dB at full
  amount, using the same `(vel − 0.78)/0.22` shape and smoothed over 30 ms
  at note boundaries (`vocal_svs/post.rs`). This is what makes TIGER
  dynamic.

The render-unit key already hashes velocity.

## 10. Expression curves as automation lanes

- **Model:** the curves are Dynamics, Tension, Breathiness and Pitch bend,
  using the existing `compose/expression.rs` model and ranges.
  - They move into the lane config (inside `VocalParams`, `#[serde(default)]`),
    so they are saved and ride undo. That fixes the curve part of BUG-50.
  - They are stored **section-relative** in beats from the section start.
    Today they are normalised to the clip.
- **Arrange:** a vocal track gets automation sub-rows
  `Vocal: Dynamics | Tension | Breathiness | Pitch bend`.
  - At every placement of a section with a vocal lane, a sub-row shows
    that lane's curve. Editing at any placement edits the section's curve,
    and every placement follows, like editing a section's MIDI. One render
    per lane stays (FU-M11b).
  - Outside placements the row is inert.
  - Curves the bank does not support are greyed out with "not supported by
    <bank>".
- **Dynamics:** drives energy where the bank supports it, otherwise the same
  post-render gain as §9. That makes it audible on every bank.
- **Target:** a new `AutomationTarget::VocalCurve { track, curve }`. It is
  never sent to the engine; an edit marks the affected units stale or
  re-renders them under the §7 rule.
- **Control API:** `automation.*` accepts the target as
  `{vocal: {track_id, curve: dynamics | tension | breathiness | pitch_bend}}`.
  - Point positions are song positions. They map to placement, then
    section, then offset, and a point outside every placement is rejected.
  - `automation.lanes` lists vocal lanes with `section_relative: true`.
  - `song.vocal` lists which curves each lane has.
- **What happens to the earlier designs:** the prototype's separate dock
  (`intents/prototypes/editable-vocal-expression-curves/`) is superseded,
  and `ExpressionDockState` goes. The breakpoint model and handlers in
  `update/compose/expression.rs` are reused.

## 11. Decisions taken (Q&A 2026-10-07)

| # | Question | Answer |
|---|---|---|
| D1 | Params API | One `vocal.set_params` / `vocal.get_params` pair with partial patches |
| D2 | `set_lane_generator` re-issue | Merge into the existing lane and keep lyrics and seed |
| D3 | Wire enum names | snake_case |
| D4 | `song.vocal` readout | Full params per lane |
| D5 | Melisma | `_` lyric token |
| D6 | `SyllableMode` | Implement it |
| D7 | `s`-key slurs on regenerate | Write `_` into the lyrics |
| D8 | `lines[]` index | Per lane |
| D9 | Default bank | **Lilia** for now. Resonance is not commercial yet, so Lilia's generative-AI clause and the NC-SA vocoder are accepted for now and must be revisited before any commercial release. |
| D10 | Bank facts | A hardcoded table plus a disk check, and later: derive first, built-in overrides second |
| D11 | Scope | **Includes user-installed DiffSinger banks** |
| D12 | Missing bank | The render fails, naming the bank |
| D13 | Bank / speaker id | A string id and one `speaker` field |
| D14 | Distribution | The plok.org index points at upstream URLs; no copies |
| D15 | Library UI | An overlay from the lane inspector |
| D16 | MCP install | Yes, `vocal.voicebank_install` as a job |
| D17 | Vocoders | Index entries, fetched automatically |
| D18 | Phonemes for arbitrary banks | The bank dictionary, then CMU mapped onto its inventory |
| D19 | Staleness | Compare plan hashes |
| D20 | Auto-render | Debounced in the GUI, explicit over MCP |
| D21 | Chord edit | Same rule as other edits |
| D22 | While stale | The old audio plays, with a badge |
| D23 | User breaks | Exact labels and phoneme split |
| D24 | Pronunciation source | A per-note `g2p` enum |
| D25 | Rest threshold | Beat-relative, default ½ beat |
| D26 | Breath | `AP` token in the gap, taking no note |
| D27 | Velocity | Bank curves plus a post-render gain |
| D28 | Expression curves | Automation sub-rows with a render-only target, section-relative |
| D29 | Absorbed into this spec | Pronunciation persistence, expression curves, `resonance-svs` fixes, `curve_supported` fix |
| D30 | Bank audition | A blind render script |
| D31 | SoulX-Singer | A separate intent ([soulx-singer-backend](soulx-singer-backend.md)) |
| D32 | Spec home and landing | This document; each slice lands on master separately |

Pronunciation persistence (slice 11) keys per-syllable overrides by
`(definition, track, syllable index)` and drops an override whose syllable
text changed. That is the existing intent's recommendation and was not
asked again.

## 12. Slices

Each slice is a vertical slice that lands on master on its own: wire,
handler, MCP tool, GUI, tests, and the feature doc with its
`Verified against master @` line (see
[control-API conventions](../../guides/control-api-conventions.md)).

1. **Default and description fixes.** ✅ built (phase 1)
   - Lilia becomes the code default (`params.rs:663`, `VoiceType::default_singer`).
   - The Lilia remap in `vocal.render` goes (`update/control/vocal.rs:428-432`,
     `:633`), which fixes BUG-52.
   - The wrong "four vocal tracks" text is fixed (`arrange.rs:134-136`,
     `section.rs:169-177`), along with every description that names a
     default bank.
   - `lines[]` moves to per lane.
2. **`resonance-svs` honesty.** ✅ built (phase 1). Measured: the shipped acoustic models take `steps`, so `speedup` 50 is 1.7× *slower*; default stays 20.
   - `curve_supported` reads the `use_*_embed` flags.
   - The `rocm` feature is replaced by MIGraphX (ONNX Runtime 1.23 removed
     ROCm).
   - A requested GPU backend uses `error_on_failure`, and the active backend
     is logged and reported in `control.hello`.
   - Measure `speedup` 20 against 50 (render time and a listen) with
     `examples/render_resonance_vocal.rs`, then decide.
3. **Params API.** ✅ built (phase 1). The wire patch and view types, `vocal.get_params`,
   `vocal.set_params`, the `set_lane_generator` merge, `song.vocal` params,
   snake_case, and `PROTOCOL_VERSION` 2 with the lockstep update.
4. **Bank registry.** ✅ built (phase 2; see
   [vocal rendering](../../features/compose/vocal-rendering.md)). Deviations:
   `vocal.set_params` accepts installed banks plus the three built-in ids
   until the index exists; `vocal.generate` reports the missing bank as a
   `render_error` field of its result rather than failing (the melody is
   written either way).
   - The string id and `speaker`, with old-project mapping.
   - `VoicebankRegistry` with derived facts and the built-in table, and the
     user voicebank and vocoder directories.
   - The `voicebank_missing` error.
   - `vocal.voicebanks`, installed list only.
5. **Any-bank phonemisation.** ✅ built (phase 2; see
   [pronunciation](../../features/compose/pronunciation.md)). Deviation: bank
   dictionary entries that use non-ARPAbet symbols (`dx`, Lilia's `L`) are
   skipped, so those words fall back to CMU. The bank dictionary, then CMU mapped onto
   the inventory; generalised substitutions and language ids; the
   `english` flag.
6. **Bank audition script.**
   - A `resonance-svs` example renders one committed test song on every
     installed bank and speaker into `target/audition/`, with names hidden
     behind a key file.
   - Jorrit listens blind, and the results pick which banks go in the index:
     Peiton, LUNAI Katyusha / Keiro / Liam and Meiji v170 against the
     current three.
7. **plok.org install.** Index entries, shared `plok_download`, the library
   overlay, `vocal.voicebank_install`, and the available list in
   `vocal.voicebanks`.
8. **Melisma.** Library side (`_`/`+` holds, coda on the last hold, counts) built in phase 1; The `_` token, counts, the `s` key editing the draft,
   `SyllableMode`, and `slur: user | auto`.
9. **Render state.** ✅ built (phase 3; see
   [vocal rendering § render state](../../features/compose/vocal-rendering.md#render-state)).
   Deviations: the GUI debounce re-renders only lanes that already have
   audio, a render in flight or a failed render (a never-rendered lane
   waits for Generate / Re-render), and not when the edit changes no unit
   key; the plan memo is keyed by a fingerprint of the plan's inputs, not
   a lane revision counter; the top-level order is `error > rendering >
   stale > not_rendered > rendered`; the badge also shows `RENDERING` and
   an error card; a take saved before keys existed reads `stale`; chord
   and length edits no longer render at all (they re-derive the melody
   and leave the lane stale), which replaces plumbing the dropped Task.
   - Per-lane state from plan hashes, with keys saved with the audio.
   - The debounced GUI re-render and the stale badge.
   - The chord-edit Task fix.
10. **Syllables and pronunciation source.** Library side built in phase 1 (exact breaks, `G2pSource`, corpus); Exact user breaks, `g2p` per
    note, and the corpus fix and test.
11. **Pronunciation persistence.** ✅ built. The project dictionary and
    overrides in `ProjectFile::pronunciation`; the `pronunciation`
    reconcile domain restores them instead of clearing them. Overrides are
    keyed by `(definition, track, syllable index)` with the syllable text,
    and ignored when the text changes. This fixes BUG-50's pronunciation
    part. See [pronunciation](../../features/compose/pronunciation.md).
12. **Rests and breaths.** `rest_threshold_beats`, the `AP` token, the AP/SP
    check, and re-blessed goldens.
13. **Velocity dynamics.** `velocity_dynamics`, the bank curve, and the
    post-render gain.
14. **Expression curves as automation.** The curves in the lane config,
    section-relative storage, Arrange sub-rows, `AutomationTarget::VocalCurve`,
    and `automation.*`. This fixes BUG-50's curve part.

Slices 1–3 come first, because they unblock the field report's P1. Slice 4
must land before 5–7, and 8–14 are independent of each other.

## 13. How we'd know it works

- **Control tests** (`resonance-app/tests/control/`):
  - `set_params` with one field leaves the rest unchanged, and
    `song.vocal` echoes it;
  - an invalid value is rejected at set time;
  - a `set_lane_generator` re-issue keeps the lyrics;
  - snake_case round-trips;
  - `vocal.voicebanks` lists a temp-dir bank and drops it once it is
    deleted, and rendering with it then fails with `voicebank_missing`;
  - a lyric edit makes only that lane `stale`;
  - `love _ _` gives `counts_mismatch: false` and survives `vocal.generate`.
- **Install tests:** a local HTTP server serves an index and a zipped
  fixture bank plus vocoder, like the drums' `download.rs` tests, covering
  the sha256 mismatch and the vocoder auto-fetch.
- **G2P tests** (`resonance-music-theory/tests/g2p*.rs`):
  - `fo·rget` splits as written;
  - a rules fallback reports `g2p: rules`;
  - `_` gives slur tokens and `AP` gives a breath token;
  - every corpus word resolves through CMU.
- **Render tests** (`resonance-app/tests/vocal/`): each scenario pins its
  params and guards against silence on its own, because a silent golden
  matches forever.
  - an eighth-note rest at the default threshold gives a silent gap;
  - `AP` gives a breath segment;
  - on TIGER, a velocity sweep changes RMS per note;
  - a Dynamics curve changes the render on a bank without energy;
  - curve support follows the `dsconfig` flags.
- **Persistence:** set a pronunciation and a curve, save, reopen and undo
  an unrelated edit. Both are still there, and the lane reads `rendered`.
- **UI goldens:** the library overlay states, the stale badge, and the
  vocal automation sub-rows (`tests/snapshots/`).
- **Live:** an MCP session installs a bank, sets params, writes a melisma
  and breaths, renders, and nothing goes through the GUI.
