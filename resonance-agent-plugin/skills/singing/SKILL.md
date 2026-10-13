---
name: singing
description: Write and render a sung vocal on the singing-voice synth in a running resonance DAW — choosing and installing a voicebank, one vocal lane per section, a melody (generated or hand-written), singable lyrics with exact syllable breaks, holds and breaths, the song_vocal pre-flight, one batched render, expression curves and an intelligibility check. Use when a song needs a vocal, lyrics, a sung melody, a guide vocal, a different singer, or when a vocal sounds wrong, mumbled, out of range or out of date.
when_to_use: >-
  Triggers on requests like "add a vocal", "write lyrics for the chorus",
  "make it sing", "put a guide vocal on it", "use a different voice", "the
  words are mumbled", "the vocal is too high", "hold that note longer", "add a
  breath before the line", "re-render the vocal", "install a voicebank".
allowed-tools: mcp__resonance__control_hello mcp__resonance__song_summary mcp__resonance__song_sections mcp__resonance__song_tracks mcp__resonance__song_notes mcp__resonance__song_vocal mcp__resonance__vocal_get_params mcp__resonance__vocal_voicebanks mcp__resonance__automation_lanes mcp__resonance__job_status mcp__resonance__job_wait
---

# Singing in resonance

A vocal here is a MIDI melody plus lyrics, sung by a neural voicebank. You
cannot hear it, but `song_vocal` comes close: it reports, per note, the
syllable, its phonemes, whether the note is long enough to say them, whether
it sits in the singer's range, and whether the audio is current. Read it after
every edit and before every render.

The order below is the order to work in. Nothing in it assumes a genre, a track
name or a track count: read the song first, then decide.

## 0. Preflight

Call `mcp__resonance__control_hello`. This procedure needs `song.vocal`,
`section.set_lane_generator`, `vocal.get_params`, `vocal.set_params`,
`vocal.set_lyrics`, `vocal.set_line`, `vocal.generate` and `vocal.render`. If
any is missing, the running app is older than this plugin — say so and stop.

The later passes need more; check them and skip the pass (saying why) when one
is missing:

| Pass | Needs |
|---|---|
| Choosing or installing a voice (1) | `vocal.voicebanks`, `vocal.voicebank_install` |
| A hand-written melody (3) | `notes.replace_all`, `notes.import_midi` |
| Changing chords under a melody (3) | `vocal.refit_melody` |
| Fixing a word's pronunciation (5) | `vocal.set_pronunciation` |
| Expression curves (7) | `automation.set_lane`, `automation.add_points`, `automation.lanes` |
| The intelligibility check (8) | `render.stems`, or else `mixer.set_solo` and `render.mixdown` |

Then read:

```
mcp__resonance__song_summary     # tempo/meter events, tracks, revision
mcp__resonance__song_sections    # definitions, placements, chord grids
mcp__resonance__song_vocal       # existing vocal lanes, if any
```

**Sections and chords come first.** The melody generator walks the section's
chord grid and does nothing without one. A chord or length edit *after* the
melody keeps the lane's notes by default and flags the lane
`melody_predates_chords` in `song_vocal` — they now clash with the new
chords until you deal with them (step 3). No
sections: `${CLAUDE_PLUGIN_ROOT}/skills/song-structure/SKILL.md`. No chords:
`${CLAUDE_PLUGIN_ROOT}/skills/arranging/SKILL.md`, step 1. Settle both before
writing a note of melody.

## 1. Choose the voice

`mcp__resonance__vocal_voicebanks` lists what is `installed` and what is
`available` to download. A new lane sings TIGER's `mystic`, the best-rated
voice in the house listening tests; keep it unless the song asks for
something else.

**Start from a voice preset.** `mcp__resonance__vocal_voice_presets` lists
the built-ins (every voice the blind listen scored 3 or more, best first,
each with its `score`) and the user's own. Each one sets the bank, speaker,
voice type, range and delivery in one undoable step through
`mcp__resonance__vocal_apply_voice_preset`, and leaves lyrics, melody
settings and curves alone, so a finished part can change singer. Pick by
register and character from its `tags`, then fine-tune with
`vocal_set_params`. A setup the user likes can be kept with
`mcp__resonance__vocal_save_voice_preset`.

- **Range decides more than timbre.** Each bank and speaker has a
  `comfortable_range`. Pick the voice whose range holds the melody's centre,
  or move the melody into the range (step 3). A voice pushed past its range
  is the most common reason a render sounds strained or unclear.
- **An `effect` speaker is not a lead voice.** Growl, vocal fry (`vf`) and
  whisper colour a word or a line; they do not carry a melody.
- **Curves and models differ per bank.** `curves` says which expression
  curves reach the model; `models` says whether `bank_timing` / `bank_pitch`
  have anything to use.
- **A different singer is a different level.** Two voices singing the same
  notes can differ by 10 dB or more. After `vocal_apply_voice_preset` or a
  bank switch, render and re-measure the part before judging it, and re-set
  its fader.
- **Installing is the user's call.** `vocal_voicebank_install` downloads
  hundreds of MB and accepts the bank's terms on the user's behalf. Tell
  them the `credit`, `terms_url` and `commercial` terms first, install only
  when they agree, and wait on the job with `job_wait`.
- **Always pass the credit on.** When you use a bank, tell the user its
  `credit` line and what `commercial` means for a release.

`${CLAUDE_SKILL_DIR}/references/voicebanks.md` has the banks, their ranges and
terms, which voices scored well, and how to match a voice to a song.

## 2. Lanes: one per (section, vocal track)

A vocal track sings through **lanes**, one per section definition. Lyrics,
melody and voice settings belong to the lane, and every placement of that
section sings the same thing. So:

- Verse 1 and verse 2 with different words need **two section definitions**,
  even if the music is identical (`section_create`, then apply the same
  chords with `harmony_apply_progression`).
- `track_add {kind: "vocal"}` if there is no vocal track. Then
  `section_set_lane_generator {section_id, track_id, kind: "vocal"}` once per
  section that sings. `options` is a partial settings patch; on a lane that
  exists it merges and keeps the lyrics and seed.
- Address every later call with that section as `section_id`. Omitted, most
  `vocal_*` tools hit the track's **first** lane, which is rarely the one you
  meant.
- Change settings with `vocal_set_params` (a partial patch, one undo step)
  and read them with `vocal_get_params`. Neither renders.

## 3. The melody

Two ways in, chosen per lane:

- **Generate.** `vocal_generate {section_id, lyrics: false}` writes a melody
  over the chords and returns the lane's `clip_id`. **Pass `lyrics: false`
  whenever you have written or will write the lyrics** — the default
  replaces them with template lines. Before generating, set the generator's
  `voice`, `range` (inside the voicebank's `comfortable_range`), `style`,
  `contour` and `syllable_mode` with `vocal_set_params`. `syllabic` sings one
  note per syllable; `mixed` and `melismatic` add generator holds (`slur`
  `auto` in `song_vocal`), which the next generate replaces. Change `seed` on
  purpose to hear another take.
- **Write it.** When the tune is the point, generate once to get the clip,
  then replace its notes: `notes_replace_all` (with `confirm: true`) or
  `notes_import_midi` for a long line. The lane sings the clip
  `vocal_generate` returned (its first placement) at every placement. A later
  `vocal_generate` overwrites it. A chord edit or section resize keeps it;
  pass `vocal_melody: "refit"` to the `harmony_*` / `section_resize` call to
  move its pitches onto the new chords (rhythm, lyrics and holds stay), or
  run `vocal_refit_melody` afterwards on a lane `song_vocal` reports with
  `melody_predates_chords: true`. `vocal_melody: "regenerate"` replaces it.
  The reply's `vocal_lanes_affected` says what each lane got.
- **Every placement sings the first placement's clip.** Each placement of
  the section gets its own MIDI clip, but the lane renders once, from the
  first one, and lays that take on every placement. Write and fix notes in
  that clip; the others keep whatever they were created with (often the raw
  generator output — a reason a second placement *looks* wrong in
  `song_notes` while it sings right). Emptying the first clip makes the lane
  refuse to render, so silencing one placement is a track volume lane, not a
  note edit (`references/backing-vocals.md`).

Either way: one note per syllable or hold, stressed syllables on strong beats
and long notes, a rest between phrases, and every note inside the voice's
range.

**Timing that sounds sung, not sequenced.** A singer does not land every
syllable on the grid, and does not space quick syllables evenly:
- Keep each phrase's first word and its held end where the music wants
  them; move the syllables *inside* the phrase by a few tens of ms (about
  -25..+65 ms, mostly late for a laid-back feel), and let the spacing
  between them vary.
- A syllable under about a third of a beat (≈250 ms) is heard as clipped.
  Give it room by borrowing time from its neighbours inside the phrase, not
  by moving the phrase — and never leave the result evenly spaced: that is
  what makes a line sound programmed.
- A line's first syllable on the downbeat of every line is a tell too; vary
  the entries (the "and" of a beat is often right).
- After a timing pass, run the intelligibility check: a syllable moved
  late or shortened can turn into another word ("on" heard as "up"); put
  that one note back. Transposing a written melody into range is a `notes_replace_all` with
every pitch moved — by an octave when the key must stay.

## 4. The lyrics

`vocal_set_lyrics {section_id, text}` writes a lane, one line per `\n`;
`vocal_set_line {section_id, line_index}` replaces one line (`line_index` is
0-based **within the lane**, as `song_vocal` lists it).

The syntax in one breath: `·` (or `-`) inside a word is an exact syllable
break, one note per piece; a standalone `_` (alias `+`) holds the previous
syllable onto one more note; a standalone `AP` is an audible breath before the
next word and takes no note; `[..]` holds raw phonemes. Unbroken words are
split for you while `syllabify` is on.

Writing words a synth can sing clearly is its own craft — speech rhythm
first, open vowels on long notes, time for consonant clusters, no crammed
syllables. Read `${CLAUDE_SKILL_DIR}/references/lyrics.md` before writing the
first line; it has the craft and the full syntax with examples.

## 5. Pre-flight with `song_vocal`

After every lyric, note or settings change, read `song_vocal` and fix in this
order:

1. **`counts_mismatch`** — the lyric fills a different number of notes than
   the lane has. Trust `syllable_count`, not your own count. Fix with breaks,
   holds or notes until it is `false`.
2. **`too_short`** (`short_note_count`) — a note shorter than its phonemes'
   `min_duration_ms`. It will be sung but not understood. Lengthen the note,
   move a syllable, or simplify the word; no synthesis setting fixes it.
3. **`out_of_range`** (`out_of_range_note_count`) — against the lane's
   `comfortable_range`. Move the melody or pick another voice.
4. **`syllable_warning`** — your `·` pieces do not match the word's vowels,
   so it was split automatically. Re-break the word.
5. **`g2p`** per note — `rules` means no dictionary knew the word and the
   pronunciation is a spelling guess. Check its `phonemes`; fix a wrong one
   with `vocal_set_pronunciation` (project-wide ARPAbet).
6. **`slur`** — `user` is a `_` you wrote, `auto` a generator melisma. A hold
   you want to keep belongs in the lyrics.

Only when all of these read clean is the lane worth rendering.

## 6. Render once, after the batch

Edits over MCP **never** re-render. Each one leaves the lane `stale` (the old
audio keeps playing) until `vocal_render`. So batch every edit for a lane,
pre-flight it, then render once:

- `vocal_render {section_id}` renders one lane; `{track_id}` every lane on
  the track in one job. It is a job: if the tool returns still-running, block
  on `job_wait`.
- Read `song_vocal` again: each lane's `render_state` should be `rendered`.
  `stale` with `stale_units` means an edit landed after the render;
  `rendering` means wait; `error` carries a `kind` — `voicebank_missing`
  (install the bank or pick an installed one, step 1), `no_notes` (generate
  or write the melody), `nothing_to_sing` (the lyrics resolved to no
  phonemes), `unsupported_phonemes` (a `[..]` block or pronunciation the bank
  cannot sing), `invalid_params` (re-read `vocal_get_params`).
- `vocal_generate` is the exception: it queues a render itself, and reports
  `render_error` when the bank is missing.
- Renders are deterministic: the same lane renders the same audio every
  time, so re-rendering unchanged material changes nothing. Change the
  melody, the words or the settings, not the dice.
- **Re-render after arrangement edits.** A take starts a little before its
  section (the singer's pre-roll), so `arrangement_insert_bars` /
  `arrangement_remove_bars` at a section boundary can cut it or leave it
  behind, and a section placed again (`section_place`) can read `rendered`
  in `song_vocal` with no take on it. After any of these, `vocal_render
  {track_id}` and confirm in `song_tracks` that every placement has its
  audio clip.

## 7. Expression

Only after the words and notes are right. In order of reach:

- **`rest_threshold_beats`** (default 0.5) — the shortest gap sung as
  silence; shorter gaps are sung legato. Raise it for a smoother line, lower
  it when written rests are being sung through.
- **`velocity_dynamics`** (0..1, default 0.5) — how far note velocity moves
  loudness, on every bank. Vary velocities by phrase stress: it is the
  cheapest phrasing there is.
- **`bank_timing` / `bank_pitch`** (default on) — the voicebank's own
  duration and pitch models. Leave them on; they sing clearer. With
  `bank_pitch` on, `vibrato`, `vibrato_rate` and `portamento_ms` are not
  heard (the model sings its own). Turn `bank_pitch` off only when the line
  needs your own vibrato or a specific glide, and check the result.
- **Curves** — `automation_set_lane` / `automation_add_points` on the target
  `{vocal: {track_id, curve}}` with `curve` one of `dynamics`, `tension`,
  `breathiness`, `pitch_bend`. Points are written at song positions but
  stored per section, so every placement of that section follows. **Check
  `vocal_voicebanks`' `curves` for the lane's bank before drawing one**: a
  curve the bank does not take is accepted, stored and silently ignored.
  When this was written `pitch_bend` worked on every installed bank,
  `tension` on every bank but TIGER, and **`dynamics` and `breathiness` on
  none** — loudness phrasing is note velocity × `velocity_dynamics`.
  Units: 0..1 with 0.5 unchanged, pitch bend ±50 cents. Curves are rendered
  into the take, so `vocal_render` after.
- **Tension is the intensity lever.** On a bank that takes it, a tension
  curve rising section by section is how a song builds: at 1.0 a line came
  out about 3 dB louder and markedly brighter (+7 dB above 2 kHz) and still
  scored 100 % intelligible. Low tension (under about 0.45) is what sounds
  breathy. Two placements of one section share the curve, so a section that
  ends both the pre-bridge and the song peaks in both places. A tension
  build also raises the lead's level and loudness range in the mix: re-check
  its fader and the master's PSR after one.

## 7b. Backing vocals

A harmony, a double or a counter-line is a second vocal track with its own
lane, written by hand. The craft — a third below rather than above, every
note checked against the chord, oblique motion, loosened timing and
velocities, its own tension, a volume lane to keep it out of a placement,
choosing a non-breathy backing voice, and levelling it about 6 LU under the
lead after every voice switch — is in
`${CLAUDE_SKILL_DIR}/references/backing-vocals.md`. Read it before writing
the first backing note.

## 8. Verify without ears

`song_vocal` is the check that always runs. For a word-level check of a
finished take, when the user wants one:

1. Get a vocal-only file. `render_stems {dir, track_ids: [<vocal track>]}`
   writes `01-<track name>.wav` into `dir` (absolute; pass `overwrite: true`
   to re-check after a fix) and its result lists the path. When
   `render.stems` is missing from `control_hello`'s capabilities (an older
   app), use `mixer_set_solo` on the vocal track, `render_mixdown` to a WAV,
   then **clear the solo**, and check the result's `soloed_track_ids` holds
   only the vocal track. Either way the file is the whole vocal track; pass
   a `range` to score one section.
2. Score it against the lyric it should sing, every placement in order:

   ```
   ${CLAUDE_SKILL_DIR}/scripts/intelligibility.py --text "<the lyric as sung>" <stem>.wav
   ```

   The lyric can be pasted in lyric syntax (breaks, holds, `AP` and `[..]`
   are stripped); `--text-file` reads it from a file and `--json` gives
   machine-readable output. It prints the word accuracy (1 − WER), what
   Whisper heard and the words it missed, and trims the silence around the
   vocal first. It needs uv on PATH (it fetches Whisper itself; the first
   run downloads about 0.5 GB) and says how to install uv when it is
   missing. If it cannot run, say so and stop here; `song_vocal` stands.
3. Read the number comparatively — before and after a fix, one voice against
   another. Singing is hard for a speech recogniser; a low score points at the
   lines to re-check in `song_vocal` (the missed words show which), not at a
   verdict. A take reported `SILENT` did not render: check `render_state`.

`meter_stems` measures the vocal's level against the mix, which is the
`mixing` skill's job, not this one's.

## 9. Guide vocals

When the user will sing it themselves later, the synth vocal is a **guide**:
name the track so (e.g. "GUIDE Vocal"), write the melody in the user's own
range (ask once if you do not know it), and when that differs from the
voicebank's range move the guide by an octave, never the song's key. Report
the lyric per section with the chord changes marked, so they can sing from
it. Their take goes on an audio track; the guide is muted, not deleted.

## 10. Hand off and report

- Before this skill: `song-structure` for the sections, `arranging` for the
  chords (and `fill_vocal_gaps` on a lead part, so an instrument answers the
  singer instead of fighting it).
- After it: `${CLAUDE_PLUGIN_ROOT}/skills/mixing/SKILL.md` treats the vocal
  as the lead role (level, tone, character), and
  `${CLAUDE_PLUGIN_ROOT}/skills/spatial/SKILL.md` gives it a room with
  20-40 ms of pre-delay so it stays in front.

Report per lane: *section → voice (bank, speaker) → lyric lines → generated or
written → `render_state`*, plus the bank's credit line and terms, and any line
you know is weak (a `rules` word, a squeezed cluster, a note at the edge of the
range). Every edit went through undo; `edit_undo` reverses one call at a time.

## Additional resources

- `${CLAUDE_SKILL_DIR}/references/lyrics.md` — singability craft and the full
  lyric syntax (breaks, holds, breaths, phoneme blocks), with examples and the
  `song_vocal` fields each mistake shows up in.
- `${CLAUDE_SKILL_DIR}/references/voicebanks.md` — the banks, speakers,
  ranges, terms and listening-test results, and how to match a voice to a song.
- `${CLAUDE_SKILL_DIR}/references/backing-vocals.md` — harmonies, doubles and
  counter-lines: writing them so they sound sung, keeping a part out of one
  placement, choosing and levelling the backing voice.
- `${CLAUDE_PLUGIN_ROOT}/skills/arranging/references/generators.md` — the
  instrumental generators, for the parts around the vocal.
