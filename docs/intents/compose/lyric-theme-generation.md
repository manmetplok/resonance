# Theme-driven lyric generation

> Stage: **intent**
> Status: **partly built**
> Area: compose
> Source: code reading 2026-10-07 — `VocalParams::{theme, pov, avoid_cliches}` are edited in the rail but unused by `generate_lyrics` (`resonance-music-theory/src/derive/vocal/lyrics.rs`).

## Why

The vocal rail invites the user to write a theme brief (up to 240
characters), pick a point of view and tick "avoid clichés". The lyric
generator ignores all three: it samples a fixed corpus of 56 template
lines by mood, rhyme scheme and syllable count only. Users get lines
unrelated to their brief, and re-rolls soon repeat. `vocal.generate`'s
docs speak of "the lane's theme brief" too.

## What it would do

Generated lyric drafts reflect the theme words, the chosen POV (pronoun
use) and avoid a known cliché list when asked, while keeping the rhyme
scheme, syllable ranges and locked lines working as now.

## What exists to build on

- `generate_lyrics`, `LyricLine`, rhyme buckets and lock anchoring in
  `derive/vocal/lyrics.rs`; syllable counting in `g2p`.
- Lane inspector handlers `set_theme`, POV and clichés toggles
  (`update/compose/lane_inspector/`).
- Feature doc: [vocal lyrics and melody](../../features/compose/vocal-lyrics-and-melody.md).

## Open decisions

1. Generator: larger tagged corpus + template slot filling (offline,
   deterministic — recommended first step) vs. an external language model
   (better text, needs network and keys; agents already write lyrics via
   `vocal.set_lyrics`).
2. If none can be met, hide the theme/POV/clichés controls until they work
   (recommended as the interim honest state).

## Slices

1. POV pronoun rewriting + cliché filter over the existing corpus; tests
   in `resonance-music-theory/tests/vocal.rs`.
2. Theme keyword matching / slot filling with an expanded corpus.

## How we'd know it works

Seeded tests: changing POV changes pronouns; with avoid-clichés on, no
line from the cliché list appears; a theme word appears in the draft when
the corpus has a matching slot.
