---
name: arranging
description: Fill a song's sections with harmony and instrumental parts in a running resonance DAW — chord grids, generated bass/lead/pad parts, hand-written MIDI, and deciding which instrument plays where. Use when writing chords, adding or rewriting a part, thickening or thinning a section, or building a song out from a structure.
when_to_use: >-
  Triggers on requests like "write chords for this", "add a bass line", "give the
  chorus a lead", "arrange this", "make the second verse different", "this
  section is too empty", "put a pad under it", "write a walking bass".
allowed-tools: mcp__resonance__control_hello mcp__resonance__song_summary mcp__resonance__song_sections mcp__resonance__song_tracks mcp__resonance__song_notes mcp__resonance__plugins_catalog mcp__resonance__track_plugin_params
---

# Arranging in resonance

Arranging is deciding **who plays what, where, and how densely**. It sits on top
of structure and underneath mixing: if the sections are wrong, fix those first;
if the parts are right but the balance is wrong, that is a mixing problem, not an
arranging one.

## 0. Preflight

Call `mcp__resonance__control_hello`. This needs `harmony.apply_progression`,
`generate.part` and `notes.insert_many`. If any is missing, the running app is
older than this plugin — say so and stop.

Read before writing:

```
mcp__resonance__song_summary     # tempo, key, revision
mcp__resonance__song_sections    # definitions, placements, chord grids
mcp__resonance__song_tracks      # what instruments exist
```

No sections? That is `${CLAUDE_PLUGIN_ROOT}/skills/song-structure/SKILL.md`
first. Chords live on section **definitions**, so there is nowhere to put them
until definitions exist.

## 1. Harmony before parts

Every generator reads the section's chord grid. `generate_part` is **refused
outright** on a section with no chords, so this is not an ordering preference —
it is a hard dependency.

```
mcp__resonance__harmony_apply_progression
```

Give **exactly one** chord source:

| Source | Example |
|---|---|
| `symbols` | `["Am7", "Dm7", "G7", "Cmaj7"]` |
| `key` + `numerals` | `{tonic: "A", scale: "minor"}` + `["i","VI","III","VII"]` |
| `key` + `preset` | `pop`, `axis` (both I V vi IV), `50s`, `doo-wop` (both I vi IV V), `pachelbel`, `andalusian`, `12-bar-blues` |

Two sources, or a numeral/preset without `key`, is rejected. The advertised
`ii-V-I` preset is unreachable — write it as numerals `["ii","V","I"]`.

Three things that bite:

- **It REPLACES the section's existing chords.** Not additive. Use
  `mcp__resonance__harmony_add_chord` / `harmony_edit_chord` to adjust one chord.
- `beats_per_chord` must be a whole number of beats and defaults to one bar per
  chord. The progression has to fit the section length exactly or it is rejected
  with the arithmetic.
- `sevenths` enriches numeral and preset voicings only — it does nothing to
  explicit `symbols`, where you spell the quality yourself.

## 2. Generate, or write by hand

The generators are good at idiom and bad at intent. Use them for material that
should sound *typical*, and write by hand when the part is the point.

```
mcp__resonance__generate_part    # role: pad | bass | lead, onto a synth track
```

**Always set `options.style`.** Without options you get the default, and for bass
that default is `RootPulse` — a literal root note on every beat, which is a
placeholder, not a bass line. `Walking` and `Motif` are the two bass styles that
produce an actual part.

`${CLAUDE_SKILL_DIR}/references/generators.md` has the full option tables per
role, which styles are worth reaching for, and where each one falls down.

Note that `chord_count`, `beats_per_chord` and `sevenths` appear in
`generate_part`'s schema but the app **ignores** them — it always reads the
section's grid as it stands. Shape harmony in step 1, not here.

`seed` makes output reproducible; omitted, it derives from the section id, so
repeating a call is stable rather than random. To get a *different* take, change
the seed deliberately.

Drums are a separate job with its own idioms and its own note map — see
`${CLAUDE_PLUGIN_ROOT}/skills/drumming/SKILL.md`.

### Writing by hand

For anything the generators cannot reach — a specific riff, a countermelody, a
voicing you actually chose:

| Tool | Use |
|---|---|
| `mcp__resonance__notes_create_clip` | Empty clip in a placement, or at an explicit 1-based `start_bar` |
| `mcp__resonance__notes_insert_many` | **Prefer this.** One undoable edit for a whole part |
| `mcp__resonance__notes_replace_all` | Rewrite a clip wholesale; destructive, drops every existing note — refuses on a non-empty clip until `confirm: true` |
| `mcp__resonance__notes_import_midi` | Bulk material — far cheaper than a JSON note array |
| `mcp__resonance__notes_insert` | One note. Rarely the right call |

**Never write a part with repeated `notes_insert`.** Each call is its own undo
entry, so a 400-note part leaves 400 of them and the user cannot step back out of
it. For a few thousand notes, the JSON array itself starts to dominate the
session cost — use `notes_import_midi` with a `path` or `data_base64` instead.

Coordinates: pitch is MIDI (60 = C4), `start_beat` is **0-based and
clip-relative**; it and `duration_beats` count **quarter notes**
whatever the meter (a 6/8 bar is 3.0), velocity 1–127. Bars in the
arrangement are 1-based, and a timeline `{bar, beat}` position's `beat` is
1-based in the **signature's** beat (an eighth in 6/8). Mixing those
conventions up is the most common way a hand-written part lands in the wrong
place.

After `notes_edit` or `notes_delete`, indices shift — **re-read
`mcp__resonance__song_notes` before the next index-based edit.**

## 3. Arrange across sections, not within one

A song where every section has the same parts at the same density is not
arranged, however good the individual parts are. The decisions that matter are
comparative:

- **What enters, and where.** A part that plays from bar 1 to the end has no
  entrance, so nothing about it can be a surprise.
- **What drops out.** The easiest way to make a chorus bigger is to take
  something out of the verse before it.
- **Register.** Two parts in the same octave fight; the fight shows up later as
  an unfixable mix. `register` on lead and pad, and `base_note` on bass, are the
  controls.
- **Density.** Same idea, busier or sparser per section. `generate_drums` has
  `density` for exactly this; for melodic parts it is `note_value_ticks` and
  `rest_density` on arp styles, or `complexity` on `Motif`.

`fill_vocal_gaps` on lead deserves a specific mention: it sounds only where the
section's vocal lane is silent, which is call-and-response for free and the
single easiest way to stop an instrumental line from fighting the singer.

## 4. Verify every part

Generators return `clip_ids` — **one per placement of the section**, in
arrangement order — and `clip_id` as the first of them. No follow-up
`song_tracks` call is needed.

Read the result back with `mcp__resonance__song_notes` on the returned
`clip_id`. You cannot hear it, so this is the only check that it is not empty,
not an octave out, and not a root note on every beat.

Watch the `revision` counter on every mutating result. If it jumps by more than
your own calls, the user is editing at the same time — re-read before continuing.

## 5. Report

List *section → track → what it plays → generated or written*, and say plainly
which parts are placeholder-grade. An arrangement handed over as "done" that
contains three `RootPulse` basses is worse than one handed over with them named.

## Additional resources

- `${CLAUDE_SKILL_DIR}/references/generators.md` — full `generate_part` option
  tables for bass, lead and pad, with which styles actually produce a part.
- `${CLAUDE_PLUGIN_ROOT}/skills/song-structure/references/forms.md` — form
  catalogue and bar math.
