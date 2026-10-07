# Pronunciation editing, dictionary and persistence

> Status: **partly built**
> Area: compose
> Source: prototype `docs/intents/prototypes/svs-pronunciation-control/index.html` (design #173, notes #174; todos #494–#500).

## Why

When the CMU dictionary or the rules mispronounce a word, the only fix
today is `vocal.set_pronunciation` from an agent — and that entry is lost
on project reload **and on any undo**, because `PronunciationState` is
runtime-only and `ComposeState::load_from_project` clears it. Users have
no GUI at all: the vocal roll's phoneme strip is read-only. Per-syllable
overrides and the global (user-wide) dictionary exist as types but nothing
creates or loads them.

## What it would do

- Project dictionary saved in the project file and kept across undo.
- Interactive phoneme strip: click a syllable, remove/reorder/add
  phonemes, pick a CMU variant, split a syllable across two notes or merge
  with the next, revert to auto, add to dictionary.
- Dictionary editor with Project / Global scopes, import / export; the
  global dictionary loaded from user config at resolve time.
- Control methods for per-syllable overrides.
- Edits re-render only the affected units (already true of the cache).

## What exists to build on

- `resonance-app/src/compose/vocal_svs/pronunciation.rs`
  (`PronunciationState`, `SyllableOverride`, `DictionaryScope`,
  `resolve_clip` with full precedence), `g2p::cmu_variants`.
- `update/compose/vocal_render_plan.rs::resolve_and_validate` (passes an
  empty global dictionary today).
- Render cache: `compose/vocal_svs/render_cache.rs`.
- Tests: `tests/vocal/pronunciation_resolution.rs`,
  `vocal_segment_pronunciation.rs`.
- Feature doc: [pronunciation](../../features/compose/pronunciation.md).

## Open decisions

1. Where overrides are keyed: `(ClipId, note_index)` as now breaks when a
   regenerate reuses the clip id with different notes. Recommended: key by
   `(definition, track, syllable index)` and drop overrides whose syllable
   text changed.
2. Global dictionary file location: the app config dir, as
   `pronunciation.json` (recommended).

## Slices

1. Persist project dictionary (+ overrides) in `ProjectFile`; undo
   round-trip test.
2. Global dictionary load/save + scope in `vocal.set_pronunciation`.
3. Phoneme-strip editing in the vocal roll with goldens.
4. Dictionary editor modal, import/export.

## How we'd know it works

`vocal.set_pronunciation`, save, reopen, undo something else, render:
`song.vocal` still lists the entry and the segment uses its phonemes.
