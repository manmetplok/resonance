# Vocal control over MCP, correctness and expressiveness

> Status: **idea**
> Area: compose (vocals) + control-api
> Source: MCP field report, 2026-10-07 (an agent writing and singing a song over `resonance-mcp`).

## Why

An agent can set lyrics and pick a voicebank, and nothing else that shapes
the voice. Voice type, timbre, vibrato, tension, portamento, articulation,
the TIGER speaker, the Meiji mode and the melody and lyric generator settings
can only be changed in the GUI. The agent also cannot see what is installed,
cannot hold a syllable across several notes, and gets wrong answers from
`song_vocal` about whether a lane needs re-rendering. On top of that, some
render-side limits (no staccato, no breaths, velocity that does nothing on
TIGER) cap the quality even when the GUI is used.

Ordered by priority: P1 is what an agent needs to get good results at all,
P2 is correctness and quality, P3 is bigger expressiveness work.

## What it would do

### P1: needed for good results over MCP

1. **`vocal.set_params` / `vocal.get_params` per lane.** They cover
   everything in `VocalParams`: voice type, timbre, vibrato depth and rate,
   tension, portamento, consonant emphasis, articulation, TIGER speaker
   (Glam, Fresh, Disco, Royal, Electric, Mystic, Vinyl), Meiji mode
   (including Phantom, the whisper), the melody generator (style, contour,
   range, breath) and the lyric generator (mood, rhyme, theme).
   - Partial updates: unspecified fields keep their value. Apply the same
     rule to `section.set_lane_generator`'s vocal options, which today
     rejects an incomplete set.
   - `song.vocal` reports the current values per lane.
2. **`vocal.voicebanks`.** Lists TIGER, Lilia and Meiji with their speakers
   or modes, comfortable range, phoneme gaps (Lilia has no `v`; the
   `v` → `f` substitution applies) and whether each bank is actually
   installed. Rendering with a missing bank is an error, not a silent
   no-op.
3. **Melisma over MCP.** The engine already holds a syllable across notes
   via the `+` / `-` slur markers (`g2p/note_assign.rs`), but only the GUI's
   slur key sets them.
   - Either a lyric token that survives the lyric cleaner (`lo _ _ ove`, or
     `+`), or a `vocal.set_slurs { note_indices }` method. The lyric token is
     the recommended option (decision 2).
   - `counts_mismatch` counts slur notes as held, so a lane with a slur is
     not flagged as broken.
   - `vocal.generate` keeps existing slur markers.
   - `SyllableMode` (Syllabic / Mixed / Melismatic) is read by nothing.
     Either make the melody generator honour it or remove it.
4. **One default voicebank.** The code default and `vocal.generate`'s
   auto-render use TIGER. `vocal.render` with no voicebank, the built-in
   template and the docs say Lilia. Pick one and make the code, tool
   descriptions and docs agree. This also covers
   [BUG-52](../bugs.md) (a lane set to TIGER renders as Lilia).

### P2: correctness and quality

5. **Render state per lane.** `render_state` currently says `rendered` if any
   lane on the track has audio (`view_model/lane.rs::vocal_render_state`).
   Each lane should report its own state, and go `stale` after a note,
   lyric or chord edit. During the session `rendered` showed up straight
   after a lyric edit. A chord edit regenerates the melody but does not
   re-render it.
6. **Syllable splitting.**
   - A user-placed break (`fore·get`) is the split point, not only a
     syllable count.
   - Fix the lyric generator's corpus: `"fore·get"` in
     `derive/vocal/lyrics.rs` is a typo for `for·get` and is not in the
     dictionary.
   - `song.vocal` marks words that fell back to spelling rules instead of
     the dictionary (e.g. `g2p: "rules"`), so the agent knows which words
     to check.
7. **Pronunciation overrides persist.** They are lost on save/reload and
   wiped by undo/redo. This is already the first slice of
   [pronunciation editing](pronunciation-editing.md) and [BUG-50](../bugs.md).
   It is listed here because the MCP workflow depends on it.
8. **Breaths and rests.**
   - Any gap shorter than the note's own length plus `SILENCE_GAP_SEC`
     (0.4 s, `compose/vocal_svs/mod.rs`) is sung legato straight through,
     so staccato and short rests cannot be sung.
   - Make the threshold a lane setting, and accept a breath token (`AP`)
     in lyrics.

### P3: expressiveness, bigger work

9. **Expression curves as automation lanes.** Energy, breathiness, tension
   and pitch bend exist in the model, but they have no UI, are not saved
   and have no API. This is
   [vocal expression curves](vocal-expression-curves.md). This report adds
   one request: expose them as automation lanes, so `automation.*` can
   reach them, rather than a separate curve API.
10. **Velocity does something.** Velocity only feeds tension
    (`segment/duration.rs`), and TIGER ignores tension, so on TIGER
    velocity is inaudible. Map it to energy (dynamics) as well, at least
    on banks without tension.
11. **Tool description fixes.**
    - `section_set_lane_generator` (`resonance-mcp/src/tools/arrange.rs`,
      `resonance-control/src/methods/section.rs`) says a song that sings in
      four sections "needs four vocal tracks". That is wrong: there is one
      lane per section.
    - Several tools still name Lilia as the default (see item 4).

## What exists to build on

- Params: `resonance-music-theory/src/derive/vocal/params.rs`
  (`VocalParams`, `SyllableMode`, voicebank default near `:663`). Wire
  shape: `resonance-control/src/methods/song.rs` (`VocalParams` view,
  `render_state`, `counts_mismatch`).
- Control handlers: `resonance-app/src/update/control/vocal.rs`
  (`default_lane_voicebank`), `view_model/lane.rs`. MCP tools:
  `resonance-mcp/src/tools/vocal.rs`, `song.rs`, `arrange.rs`.
- Slurs: `resonance-music-theory/src/g2p/note_assign.rs`, `lyric_parse.rs`
  (`SLUR_MARKER`, `is_slur_lyric`). Tests in `resonance-music-theory/tests/g2p*.rs`.
- Render: `resonance-app/src/compose/vocal_svs/` (`SILENCE_GAP_SEC`,
  `segment/duration.rs`, `render_cache.rs`, `validate.rs`).
- Feature docs: [vocal lyrics and melody](../../features/compose/vocal-lyrics-and-melody.md),
  [vocal rendering](../../features/compose/vocal-rendering.md),
  [pronunciation](../../features/compose/pronunciation.md).
- Related intents: [pronunciation editing](pronunciation-editing.md),
  [vocal expression curves](vocal-expression-curves.md).
- Agent plugin skills that sing (`resonance-agent-plugin/`) must be re-read
  when new tools land (`agent_plugin_lockstep.rs`).

## Open decisions

1. **Default voicebank: Lilia (recommended).** It is the default in the
   docs and the template, it is the agreed default (no `v`, with the
   `v` → `f` substitution), and only the code and `vocal.generate`
   disagree. Alternative: TIGER, and fix the docs instead.
2. **Melisma surface: a lyric token (recommended).** It keeps a lane's
   words and holds in one string, so `vocal.set_lyrics` and
   `vocal.generate` round-trip it. `_` is the candidate, since the lyric
   cleaner strips `+`. A `vocal.set_slurs` method is the fallback if the
   token clashes with the lyric cleaner.
3. **`SyllableMode`: implement it (recommended)**, since a working
   Melismatic mode is the generator-side half of item 3. Remove it if
   that turns out to be large.
4. **Gap threshold: per lane in `VocalParams` (recommended)**, default
   0.4 s so existing renders do not change.
5. **Velocity → energy: only on banks without tension support
   (recommended)**, using `curve_supported`, so Lilia and Meiji renders
   keep their current sound.

## Slices

Each slice is a vertical slice per
[control-API conventions](../../guides/control-api-conventions.md): wire
type, handler, MCP tool and tests together.

1. Default voicebank unified (item 4, BUG-52), plus the description fixes
   (item 11).
2. `vocal.get_params` / `vocal.set_params` with partial updates, the
   `song.vocal` readout and partial `section.set_lane_generator` vocal
   options.
3. `vocal.voicebanks`, and a missing bank becomes an error.
4. Melisma token, `counts_mismatch` counting held notes, `vocal.generate`
   keeping slurs, `SyllableMode`.
5. Per-lane render state with staleness on note, lyric and chord edits,
   and a re-render after a chord edit.
6. Syllable splitting: honour user breaks, fix the corpus typo, add the
   `g2p` provenance field.
7. Gap threshold as a lane setting, and the `AP` breath token.
8. Velocity → energy.
9. (Separate intents) pronunciation persistence, expression curves as
   automation lanes.

## How we'd know it works

- Control tests in `resonance-app/tests/control/`: `set_params` with one
  field leaves the others unchanged, and `song.vocal` reads it back.
  `vocal.voicebanks` reports a removed bank as not installed, and rendering
  with it errors. A lyric with a hold token gives `counts_mismatch: false`
  and survives `vocal.generate`. A lyric edit makes only that lane `stale`.
- G2P tests: `fore·get` splits at the break, and a rules fallback carries
  the provenance flag.
- Render tests in `resonance-app/tests/vocal/`: a 0.2 s rest with a lowered
  threshold produces a silence gap in the segment, `AP` produces a breath
  phoneme, and on TIGER velocity changes the energy curve. Pin each
  scenario's params and guard against silence per scenario.
- A live MCP session: an agent sets params, holds a melisma, renders,
  and nothing has to go through the GUI.
