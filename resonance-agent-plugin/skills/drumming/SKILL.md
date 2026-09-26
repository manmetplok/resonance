---
name: drumming
description: Program drum parts in a running resonance DAW — the built-in pattern library for typical grooves, and hand-written MIDI for anything the generator cannot reach, including jazz, prog odd-metre, latin and metal. Use when writing, replacing or varying drums, adding fills, or making a groove sit differently.
when_to_use: >-
  Triggers on requests like "write a drum part", "the drums are boring", "add a
  fill", "make it a shuffle", "jazz drums", "swing this", "a 7/8 groove", "give
  the chorus bigger drums", "program a beat", "double-time the bridge".
allowed-tools: mcp__resonance__control_hello mcp__resonance__song_summary mcp__resonance__song_sections mcp__resonance__song_tracks mcp__resonance__song_notes mcp__resonance__global_list_events
---

# Drum programming in resonance

Two ways to get drums, and the choice is the whole skill:

- **`generate_drums`** — the built-in pattern library. Fast, idiomatic,
  reproducible, and limited to the grooves it ships. Reach for it first.
- **Hand-written MIDI** — anything with a specific feel: swing, odd groupings,
  linear playing, a particular fill. The library has no jazz ride pattern and no
  odd-metre grooves, so those are always hand-written.

## 0. Preflight

Call `mcp__resonance__control_hello`. This needs `generate.drums` and
`notes.insert_many` (`notes.replace_all` for rewrites). If any is missing, the
running app is older than this plugin — say so and stop.

`mcp__resonance__song_tracks` for a drums track — `generate_drums` rejects any
other kind, and `track_add {kind: "drums"}` makes one.

For odd-metre work (step 3) also check `global.list_events` and
`global.add_signature_event`. They are newer than the rest of the surface; if
they are missing, the running app predates them and the only meter changes you
get are the ones the user already put on the global track.

## 1. The pattern library

```
mcp__resonance__generate_drums   # track_id, section_id, pattern, density
```

`pattern` resolves against the project's own pattern bank first, then the
built-ins:

| Pattern | What it is |
|---|---|
| `silence` | No drums. **Pins the section drumless** — without it, an un-generated section picks up the project default |
| `halftime` | Backbeat on 3 |
| `four-on-floor` | Kick every beat |
| `industrial` | Rigid gated 16ths |
| `breakbeat` | Syncopated, ghosted |
| `blast` | Blast beat |
| `sparse` | Kick on 1, snare on 3 |
| `toms` | Tom-led, no hats |
| `build` | One-bar ramp into 16ths |
| `fill` | Tom fill onto a crash |

An unknown name is rejected with **both** lists spelled out, so a wrong guess is
self-correcting — guess and read the error rather than asking.

Two things that make the library more useful than it looks:

- **Call it once per section.** Only the named section's drums change; other
  sections keep their material. That is what gives each section its own groove.
- **`density` (0.0–1.0, default 1.0) is the arranging control.** On a built-in it
  thins the groove toward the strong beats without dropping any voice, so the
  same pattern at 0.3 / 0.6 / 1.0 across three sections reads as *one idea
  getting busier*. This is the cheapest good arranging decision available.

Built-ins install exactly as authored, so they are reproducible and `seed` does
nothing to them. Omitting `pattern` rolls the section's existing bank pattern
instead, and there `seed` does apply.

## 2. When to write by hand

The library cannot do: swing, shuffle, odd groupings, linear playing, brushes,
specific fills, or anything where the *placement* is the idea. All of those are
`notes_insert_many` into a clip.

**Use the real note map.** `${CLAUDE_SKILL_DIR}/references/drum-map.md` has all
30 pads. It is mostly General MIDI but **not entirely** — note 39 is a sidestick
rather than a clap, and notes 21–29 are extended articulations that exist in no
GM chart. Guessing from GM knowledge produces a part that plays the wrong pads
and sounds broken rather than wrong.

`${CLAUDE_SKILL_DIR}/references/grooves.md` has worked beat-position recipes for
jazz, shuffle, prog odd-metre, latin, funk and metal, plus how to swing eighths
against a straight grid.

Mechanics:

```
notes_create_clip     # in the placement, or at a 1-based start_bar
notes_insert_many     # the whole part, ONE undoable edit
```

Never build a drum part with repeated `notes_insert` — each call is its own undo
entry, and a two-bar 16th-note groove is already ~60 notes. Use
`mcp__resonance__notes_replace_all` to rewrite a clip wholesale (destructive:
on a non-empty clip it refuses with a summary until you pass `confirm: true`),
and `mcp__resonance__notes_import_midi` when the part runs to thousands of
notes.

`start_beat` is **0-based and clip-relative**, counted in **quarter notes**
whatever the meter — a 6/8 bar spans 3.0 of them, a 7/8 bar 3.5. Bars in the
arrangement are 1-based, and the `beat` of a timeline `{bar, beat}` position
(seek, loop, clip_*, the playhead in song_summary) is 1-based in the
**signature's** beat — an eighth in 6/8 — so a 6/8 bar there has beats 1–6.
Velocity is 1–127.

## 3. Odd metre: find out what the meter actually is, then choose

**First, read the signature track — do not ask, and do not assume.**

`mcp__resonance__global_list_events` returns the song's `signature_events`
(`{bar, numerator, denominator}`, denominator resolved: 8 for 7/8) and its
`tempo_events`. `song_summary` carries the same two lists, so a summary you have
already read answers this for free. The list is **never empty** — bar 1 is the
song's initial meter — so **length 1 means one meter throughout** and anything
longer tells you exactly which bars change and to what.

**`song_summary.time_signature` is still the meter at the PLAYHEAD**, not the
song's meter. It moves when the cursor moves. A 4/4 reading there is not evidence
the song is in 4/4 throughout, and it never was — the difference now is that you
have somewhere correct to look instead of a question for the user.

Then pick one of two, and say which:

**A. Put a real signature event on the track.** The default when the meter
genuinely changes.

```
global_add_signature_event   # {bar, numerator, denominator} — 1-based bar
```

Upserts by bar (a second add at the same bar replaces the first, so retrying is
safe), `numerator` 1..=32, `denominator` a resolved power of two in 1..=32.
`global_edit_signature_event` changes an existing one and `global_remove_signature_event`
takes one away; both refuse an empty bar, and remove refuses bar 1. Nothing here
is a silent no-op.

Two things to get right, both of which cost real work if you get them wrong:

- **Write the event before the notes.** Bar/beat positions you send to
  `notes_create_clip` and `notes_insert_many` are read against the meter map as
  it stands, and existing material keeps its position in *time* rather than in
  bars — so adding the event afterwards moves the drums you just wrote onto
  different bars.
- **`transport_set_time_signature` will not do this.** It rewrites the bar-1
  event only, so on a song that already changes meter it moves the opening and
  leaves the bridge alone. Use it for what the song *starts* in.

**B. Write the odd grouping inside the existing grid.** Still correct, still
useful — the right answer when the user wants the *feel* without changing the
meter: a 7/8 figure phrasing across a steady 4/4 pulse, or a part that has to
keep lining up with material that is not moving. A 7/8 groove becomes a recurring
3+2+2 sixteenth grouping inside the 4/4 grid: the music is right, the bar lines
disagree with it by design, and the pattern's period no longer matches the bar.
Say so when you do it — the phrase only realigns with the downbeat every 7 bars.
`${CLAUDE_SKILL_DIR}/references/grooves.md` has the realignment table for the
common groupings.

**Inserting or removing bars keeps the meter with the music:**
`arrangement_insert_bars` / `arrangement_remove_bars` move tempo and signature
events along with the clips and sections (the one at bar 1 stays put; an event
inside a removed span is clamped onto the cut). Insert 8 bars before the bridge
and its 7/8 event moves with it. Do not move the events again by hand — read the
result's `signature_events_moved` or `global_list_events` if you want to check.

## 4. Velocity is the difference between a part and a grid

A drum part where every hit is velocity 100 sounds like a drum machine because it
is one. Three things fix most of it:

- **Ghost notes.** Snare at 20–45 between the backbeats is what makes funk and
  breakbeats breathe. The single highest-value edit available.
- **Accent pattern.** Hats alternating roughly 100 / 70 gives the eighth-note
  pulse a shape. Downbeats louder than upbeats, always.
- **Backbeat above everything else.** Snare on 2 and 4 at 110–127.

Micro-timing matters too: pulling snares 5–15 ticks late reads as laid-back,
pushing hats early reads as urgent. At 480 ticks per quarter, that is roughly
0.01–0.03 beats.

## 5. Fills belong at the seam, not everywhere

A fill's job is to mark a transition. One at the end of the bar before a section
change; not one every four bars. `generate_drums` with `pattern: "fill"` covers
the common case; hand-write when the fill should reference the groove it is
leaving or the one it is entering.

`build` before a chorus and `fill` into it is a two-call idiom worth knowing.

## 6. Verify

`mcp__resonance__song_notes` on the returned `clip_id`. Generators return
`clip_ids` — one per placement of the section, in arrangement order.

Check specifically: that the kick and snare are on the pitches you meant, that
velocities vary, and that the part is as long as the section. You cannot hear it,
and a part written an octave off the drum map looks fine in the tool result.

Report as *section → pattern or written → density → what makes it different from
the section before it*.

## Additional resources

- `${CLAUDE_SKILL_DIR}/references/drum-map.md` — all 30 pads with MIDI notes, and
  where the map departs from General MIDI.
- `${CLAUDE_SKILL_DIR}/references/grooves.md` — beat-position recipes per genre,
  swing maths, odd-metre groupings.
- `${CLAUDE_PLUGIN_ROOT}/skills/arranging/SKILL.md` — the rest of the parts.
