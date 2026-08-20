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

## 4. Meter: the app has a signature track, this API does not

The DAW supports **per-bar time-signature changes** on its signature track, in
the global-tracks shelf above the timeline. A song can absolutely be in 4/4 and
then 7/8.

**The control surface cannot reach it.** There is no method to add, edit, read or
delete a signature event. Three consequences, all of which bite:

- **You cannot create a meter change.** Only the user can, by hand in the GUI. If
  the arrangement needs one, say so and ask — do not pretend the limitation is
  the song's.
- **`mcp__resonance__transport_set_time_signature` rewrites only the bar-0
  event.** It does not clear later changes and does not make the song
  single-meter. On a song that already changes meter, calling it changes the
  opening and silently leaves everything after the first change alone.
- **`song_summary.time_signature` is the meter at the PLAYHEAD, not the song's
  meter.** It reports whatever signature is active where the cursor sits, and
  nothing in the response indicates that meter changes exist at all. A 4/4
  reading is not evidence the song is in 4/4 throughout.

So if the user says the song changes meter, believe them over `song_summary`, and
**ask where the changes are** — the bar math in step 3 cannot be derived from
anything this API returns.

For writing odd-metre material without a signature event, you have two honest
options. Say which you took:

- **Write the odd grouping inside the existing grid** — a 7/8 riff as recurring
  3+2+2 sixteenth groups in 4/4. The music is right; the bar lines disagree with
  it, so section boundaries stop landing on downbeats and bar math has to be done
  in beats.
- **Ask the user to add the signature event**, then work in real bars. Better for
  anything where the notation matters or the passage is long.

## 5. Build it

```
transport_set_time_signature   # if not 4/4 — writes the bar-0 event only
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
