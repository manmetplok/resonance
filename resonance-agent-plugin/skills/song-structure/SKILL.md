---
name: song-structure
description: Choose a song form and lay it out as sections and placements in a running resonance DAW — pop verse/chorus, 12-bar blues, jazz AABA, classical binary/ternary/rondo, prog multi-part. Use when starting a song from nothing, restructuring one, adding or removing sections, or deciding how long a form should be.
when_to_use: >-
  Triggers on requests like "give this a structure", "start a new song", "what
  form should this be", "add a bridge", "make it an AABA", "12-bar blues",
  "extend the outro", "this needs a pre-chorus", "lay out the arrangement".
allowed-tools: mcp__resonance__control_hello mcp__resonance__song_summary mcp__resonance__song_sections mcp__resonance__song_tracks mcp__resonance__global_list_events
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

Meter and tempo *changes* (step 4) additionally need `global.list_events` and
`global.add_signature_event` / `global.add_tempo_event`. Those are newer than
the rest, so check them only when the song actually needs a change; if they are
absent the running app predates them and you are back to asking the user to add
the event by hand in the global-tracks shelf.

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

## 4. Meter and tempo changes: the global tracks

The DAW keeps **per-bar meter changes** on a signature track and **per-bar tempo
changes** on a tempo track, in the global-tracks shelf above the timeline. A song
can be in 4/4 and then 7/8, at 96 BPM and then 140.

**The control surface reaches both.** Writing a meter change used to be
impossible here and the honest answer was "ask the user to do it in the GUI";
that is no longer true, and telling a user it is would be inventing a limitation.

### Read the tracks before you compute anything

- `mcp__resonance__global_list_events` returns `tempo_events` (`{bar, bpm}`) and
  `signature_events` (`{bar, numerator, denominator}`, denominator resolved: 8
  for 7/8) for the whole song, sorted, with 1-based bars.
- **Neither list is ever empty.** Bar 1 carries the song's initial tempo and its
  initial meter and cannot be removed — so **length 1 means "no changes, one
  meter and one tempo throughout"**, and length > 1 means the song changes.
  That is the whole test, and it costs no extra call: `song_summary` carries the
  same two lists, so step 0 already answered it.
- **`song_summary.time_signature` and `tempo_bpm` are still PLAYHEAD values.**
  They report whatever is active where the cursor happens to sit, and they change
  when the playhead moves even though the song did not. A 4/4 reading there is
  *not* evidence the song is in 4/4 throughout. The new methods did not fix those
  two fields — they put a correct answer next to them. Do every bar, beat and
  duration calculation from the event lists.

The old instruction was to ask the user where the changes are. Don't: read them.
Ask only about changes the song does not have yet.

### Write a meter change with `global_add_signature_event`

`{bar, numerator, denominator}`, and the same shape for
`mcp__resonance__global_add_tempo_event` `{bar, bpm}`:

- `bar` is **1-based** and **upserts** — one event per bar, so a second add at
  the same bar replaces the first instead of stacking two there. Re-sending a
  call you are unsure landed is safe.
- `numerator` is 1..=32; `denominator` is the note value that gets the beat,
  **resolved and a power of two** in 1..=32 — 8 for 7/8, not the exponent 3.
  `bpm` is 20..=300. Out of range is **refused**, not quietly clamped.
- `mcp__resonance__global_edit_signature_event` / `global_edit_tempo_event`
  change an event that **already exists** and refuse an empty bar — they never
  create one. `mcp__resonance__global_remove_signature_event` /
  `global_remove_tempo_event` take one away, refuse an empty bar, and **refuse
  bar 1**: that is the song's initial meter/tempo and the track cannot be without
  it (edit it, or use `transport_set_time_signature` / `transport_set_tempo`).
  None of those refusals is a silent no-op, so an `ok` means it happened.
- A signature event has no `new_bar`. To move one, add at the bar you want and
  remove the old one. A tempo event can be moved with `edit`'s `new_bar` — except
  bar 1, which is what "the start of the song" means.

**`mcp__resonance__transport_set_time_signature` is still the wrong tool for a
meter change.** It rewrites the **bar-1 event only**. On a song that already
changes meter it moves the opening and leaves every later change standing —
which reads as "it did nothing". Use it for what the song *starts* in, and
`global_add_signature_event` for everywhere else. Reaching for it to put a song
into 7/8 at the bridge is the same mistake it always was; the fix is now a call,
not a request to the user.

### What moves when the map changes

**Meter and tempo map first, material second.** A meter or tempo change makes the
bars after it a different length, and existing clips, markers and automation keep
their positions in **time**, not in bars. Adding a 7/8 event at bar 17 to a
finished arrangement lands everything after bar 17 on different bars.
(`transport_set_tempo` is the one exception — changing the song's *starting*
tempo re-anchors the arrangement and clips keep their bars.)

**`arrangement_insert_bars` / `arrangement_remove_bars` carry the tempo and
signature events with the music.** Inserting 8 bars at bar 20 moves a 7/8 event
at bar 33 to bar 41, together with the bridge it was written for. The song's
opening event at bar 1 never moves. Removing bars *over* an event clamps it onto
the cut, so the music after the splice keeps its tempo and meter; if two events
of one kind land on the cut bar, the later one wins and the other is dropped.
The result says what happened (`tempo_events_moved`, `signature_events_moved`,
`tempo_events_removed`, `signature_events_removed`). **Do not repair the global
tracks by hand afterwards** — they are already right, and "fixing" them moves
the event a second time. Check with `global_list_events` if you want to see it.

### Real meter change, or grouping inside the grid?

Both are legitimate and the choice is yours to make and to state:

- **A real signature event** is now the default whenever the meter genuinely
  changes. Bar lines land where the music does, section boundaries sit on
  downbeats, and bar math is bar math.
- **The odd grouping inside the existing grid** — a 7/8 riff as recurring 3+2+2
  sixteenth groups in 4/4 — is still correct, and still the better answer when
  the user wants the *feel* without the notation: a polymetric figure over a
  steady 4/4 pulse, or a phrase that has to keep lining up with material that is
  not changing meter. The bar lines disagree with the music by design, so section
  boundaries stop landing on downbeats and bar math has to be done in beats.
  `${CLAUDE_PLUGIN_ROOT}/skills/drumming/references/grooves.md` has the
  groupings and the realignment table.

Say which you took. What you must not do is pick the grouping *because the API
cannot do the other one* — it can.

## 5. Build it

```
global_list_events             # what the two global tracks already carry
transport_set_time_signature   # the meter the song STARTS in — bar-1 event only
transport_set_tempo            # the tempo it starts at
global_add_signature_event     # ×N, every LATER meter change, addressed by bar
global_add_tempo_event         # ×N, every later tempo change
section_create {place: false}  # ×N, keep the section_ids
section_place                  # ×M, at explicit start_bars
section_set_scale              # per section, if the song modulates
```

**The global tracks come before the sections, not after.** Placements are given
in bars, and a meter or tempo change written afterwards moves every bar past it.

`mcp__resonance__section_set_scale` takes `{tonic, scale}` with `scale` one of
`chromatic, major, minor, dorian, phrygian, lydian, mixolydian, locrian,
"harmonic minor", "melodic minor"`. Set it: `generate_part`'s `Walking` bass and
every roman-numeral progression read it, and without it they have nothing to work
from.

To insert or remove time inside an existing arrangement, use
`mcp__resonance__arrangement_insert_bars` / `arrangement_remove_bars` rather than
re-placing everything by hand. They move tempo and signature events with the
music, so the global tracks need no repair afterwards (step 4).

## 6. Verify

`mcp__resonance__song_sections` and confirm, out loud, that every placement
landed where you intended and nothing overlaps. Placement bugs are invisible
until parts are on top, and much more expensive to fix then.

If you touched the global tracks, read them back with `global_list_events` too.
You cannot hear a meter change, and an event one bar off looks exactly like an
event in the right place until parts are written against it.

Report the form as a bar map — *section → start bar → length*, plus any meter or
tempo change and the bar it takes effect at — so the user can see the shape
without opening the GUI.

## Next

Structure is the frame, not the song. Hand off to:

- `${CLAUDE_PLUGIN_ROOT}/skills/arranging/SKILL.md` — chords and parts into the
  sections you just made.
- `${CLAUDE_PLUGIN_ROOT}/skills/drumming/SKILL.md` — the kit.

## Additional resources

- `${CLAUDE_SKILL_DIR}/references/forms.md` — the form catalogue with bar counts.
