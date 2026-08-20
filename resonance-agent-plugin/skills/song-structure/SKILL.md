---
name: song-structure
description: Choose a song form and lay it out as sections and placements in a running resonance DAW — pop verse/chorus, 12-bar blues, jazz AABA, classical binary/ternary/rondo, prog multi-part. Use when starting a song from nothing, restructuring one, adding or removing sections, or deciding how long a form should be.
when_to_use: >-
  Triggers on requests like "give this a structure", "start a new song", "what
  form should this be", "add a bridge", "make it an AABA", "12-bar blues",
  "extend the outro", "this needs a pre-chorus", "lay out the arrangement".
allowed-tools: mcp__resonance__control_hello mcp__resonance__song_summary mcp__resonance__song_sections mcp__resonance__song_tracks
---

# Song structure in resonance

Form first, harmony second, parts last. Sections are the frame everything else
hangs on: chord grids belong to section *definitions*, and `generate_part` and
`generate_drums` work one section at a time. Getting the frame wrong means
rebuilding everything inside it.

## 0. Preflight

Call `mcp__resonance__control_hello`. This needs `section.create`,
`section.place` and `transport.set_time_signature`. If any is missing, the
running app is older than this plugin — say so and stop.

Then `mcp__resonance__song_summary` and `mcp__resonance__song_sections`. If the
song already has structure, you are editing, not creating: read what is there
before proposing anything.

## 1. Definitions vs. placements — the model

Two different things, and nearly every structural mistake is confusing them:

- A **definition** is a named block with a length in bars, an optional key/scale,
  and **the chord grid**. Created by `mcp__resonance__section_create`.
- A **placement** is one occurrence of a definition on the timeline, at a 1-based
  start bar. Created by `mcp__resonance__section_place`.

One definition, many placements — that is how Verse 1 and Verse 2 are the same
verse. It also means:

- Editing a definition's chords or length changes **every** placement of it. If
  verse 2 needs different chords, it needs its own definition.
- `mcp__resonance__section_remove_placement` takes one occurrence off the
  timeline; `mcp__resonance__section_delete` destroys the definition *and* every
  placement, and refuses until `confirm: true`.
- `generate_part` / `generate_drums` return `clip_ids` — **one per placement**,
  in arrangement order. Generating once fills every occurrence.

## 2. The placement trap

`section_create` places the section by default, right after the last existing
placement.

**Always pass `place: false` when building an arrangement in a deliberate
order.** Otherwise the section is already sitting at a bar you did not choose,
and your follow-up `section_place` is rejected as overlapping. The reliable
pattern is:

1. `section_create {place: false}` for every section, collecting `section_id`s.
2. `section_place` each one at its 1-based `start_bar`.

Leave `place` at its default only when appending strictly front-to-back and
"after the last one" is exactly where you want it.

## 3. Pick a form

Do not default to verse/chorus because it is the first thing that comes to mind.
Ask what the material wants: how many distinct ideas exist, whether it needs to
develop or to repeat, and how long it should run.

`${CLAUDE_SKILL_DIR}/references/forms.md` has the catalogue — pop and rock
families, 12-bar blues and its variants, jazz AABA and rhythm changes, classical
binary/ternary/rondo/sonata, and prog multi-part — each with typical bar counts
and what the form is actually *for*.

State the form and the bar math before creating anything:

> AABA, 8 bars each, 32 total, at 120 BPM in 4/4 ≈ 64 s. A at bars 1, 9 and 25;
> B at bar 17.

Then check it: placements must not overlap, and bar N of a section that starts at
bar S is `S + N - 1`. Off-by-one here is the most common structural bug, and it
does not surface until parts are generated on top of it.

## 4. Meter is global

`mcp__resonance__transport_set_time_signature` sets **one time signature for the
whole project**. There is no per-section meter.

For anything that changes meter mid-song — most prog, much classical — you have
two honest options, and you should tell the user which you took:

- **Pick the dominant meter** and write the odd-metre passages as groupings
  *inside* that grid (a 7/8 riff as recurring 3+2+2 sixteenth groups in 4/4).
  The music is right; the bar lines disagree with it, so section boundaries stop
  landing on downbeats and the bar math in step 3 has to be done in beats.
- **Set the odd meter globally** and write the 4/4 passages inside it instead.
  Better when the odd metre dominates.

Do not silently pick one. This is a real limitation of the surface, not a
detail.

## 5. Build it

```
transport_set_time_signature   # if not 4/4
transport_set_tempo
section_create {place: false}  # ×N, keep the section_ids
section_place                  # ×M, at explicit start_bars
section_set_scale              # per section, if the song modulates
```

`mcp__resonance__section_set_scale` takes `{tonic, scale}` with `scale` one of
`chromatic, major, minor, dorian, phrygian, lydian, mixolydian, locrian,
"harmonic minor", "melodic minor"`. Set it: `generate_part`'s `Walking` bass and
every roman-numeral progression read it, and without it they have nothing to work
from.

To insert or remove time inside an existing arrangement, use
`mcp__resonance__arrangement_insert_bars` / `arrangement_remove_bars` rather than
re-placing everything by hand.

## 6. Verify

`mcp__resonance__song_sections` and confirm, out loud, that every placement
landed where you intended and nothing overlaps. Placement bugs are invisible
until parts are on top, and much more expensive to fix then.

Report the form as a bar map — *section → start bar → length* — so the user can
see the shape without opening the GUI.

## Next

Structure is the frame, not the song. Hand off to:

- `${CLAUDE_PLUGIN_ROOT}/skills/arranging/SKILL.md` — chords and parts into the
  sections you just made.
- `${CLAUDE_PLUGIN_ROOT}/skills/drumming/SKILL.md` — the kit.

## Additional resources

- `${CLAUDE_SKILL_DIR}/references/forms.md` — the form catalogue with bar counts.
