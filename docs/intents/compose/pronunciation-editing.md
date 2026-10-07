# Pronunciation editing, dictionary and persistence

> Status: **partly built**
> Area: compose
> Source: prototype `docs/intents/prototypes/svs-pronunciation-control/index.html` (design #173, notes #174; todos #494–#500).

## Why

When the CMU dictionary or the rules mispronounce a word, the only fix
today is `vocal.set_pronunciation` from an agent (saved with the project
and undoable since slice 11 of
[vocal MCP control and quality](vocal-mcp-control-and-quality.md)). Users
have no GUI at all: the vocal roll's phoneme strip is read-only.
Per-syllable overrides are persisted but nothing creates them, and the
global (user-wide) dictionary exists as a type but nothing loads it.

## What it would do

- Interactive phoneme strip: click a syllable, remove/reorder/add
  phonemes, pick a CMU variant, split a syllable across two notes or merge
  with the next, revert to auto, add to dictionary.
- Dictionary editor with Project / Global scopes, import / export; the
  global dictionary loaded from user config at resolve time.
- Control methods for per-syllable overrides.
- Edits re-render only the affected units (already true of the cache).

## What exists to build on

- `resonance-app/src/compose/vocal_svs/pronunciation.rs`
  (`PronunciationState` with per-lane, per-syllable overrides that check
  the syllable text, `SyllableOverride`, `DictionaryScope`, `resolve_clip`
  with full precedence), `g2p::cmu_variants`.
- Persistence: `ProjectFile::pronunciation`
  (`resonance-app/src/project/pronunciation.rs`), restored by the
  `pronunciation` reconcile domain.
- `update/compose/vocal_render_plan.rs::resolve_and_validate` (passes an
  empty global dictionary today).
- Render cache: `compose/vocal_svs/render_cache.rs`.
- Tests: `tests/vocal/pronunciation_resolution.rs`,
  `vocal_segment_pronunciation.rs`.
- Feature doc: [pronunciation](../../features/compose/pronunciation.md).

## Open decisions

1. Global dictionary file location: the app config dir, as
   `pronunciation.json` (recommended).

## Slices

1. Global dictionary load/save + scope in `vocal.set_pronunciation`.
2. Phoneme-strip editing in the vocal roll with goldens (creates
   per-syllable overrides through `PronunciationState::set_override`).
3. Dictionary editor modal, import/export.

## How we'd know it works

A word added to the global dictionary is used in a second project; a
phoneme edited in the strip is sung on the next render and survives a
reopen.
