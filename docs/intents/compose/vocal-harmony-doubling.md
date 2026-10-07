# Vocal harmony and doubling generator

> Status: **designed**
> Area: compose
> Source: prototype `docs/intents/prototypes/vocal-harmony-doubling-generator/index.html`.

## Why

A single synthesized lead sounds thin. Producers stack harmony voices
(thirds/sixths above, SATB) and unison doubles. Today a user or agent has
to create extra vocal tracks, copy notes by hand, transpose them, and
render each one; nothing in Compose derives harmony from a lead.

## What it would do

From a chosen lead vocal lane:

- **Harmony:** N voices (above / around / below) with voicing presets
  (3rd/6th, SATB stack, custom), snapped diatonically to the chord track or
  section chords; when there is no harmonic context, offer the lead's
  detected key.
- **Doubling:** N unison doubles with per-voice timing and pitch humanize,
  stereo width, optionally an alternate voicebank.
- Voices land as editable vocal lanes/clips inheriting the lead's lyrics
  and phonemes, each re-renderable; deterministic for a seed; a "bounce
  stack" action.

## What exists to build on

- Vocal lanes, derived clips and the SVS render path
  ([vocal rendering](../../features/compose/vocal-rendering.md)).
- `satb_voicings` and `voice_lead` in `resonance-music-theory`; section
  chords and pinned chord-track harmony (`apply_chord_track_harmony`).
- Pronunciation resolution shared per clip.

## Open decisions

1. Where voices live: extra vocal tracks (recommended — each gets its own
   mixer strip and pan) vs. extra lanes on one track.
2. Lyric/phoneme inheritance: copy at generate time (recommended) vs. live
   link to the lead.

## Slices

1. Theory: `derive_vocal_harmony(lead_notes, chords, scale, voicing)` in
   `resonance-music-theory` with rule tests.
2. App: generate harmony lanes onto new tracks + render; control method
   `vocal.generate_harmony`.
3. Doubles with humanize; rail UI per the prototype; goldens.

## How we'd know it works

Theory tests (voices stay diatonic, no crossings with the lead, parallel
motion limits); a control test creates N tracks whose notes follow the
lead's rhythm; renders succeed per voice.
