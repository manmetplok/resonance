# Backing vocals: harmonies, doubles and counter-lines

A backing vocal is a second vocal track with its own lane in the section it
sings. It is written by hand (generate once for the clip, then
`notes_replace_all`), and it fails in two opposite ways: it shadows the lead
so exactly that it sounds pasted on, or it wanders off and fights it. Both
show up in the notes before any render.

## Which part

| Part | What it is | Lyrics | Works best |
|---|---|---|---|
| **Harmony** (a "double") | the lead's words and rhythm a third away | the lead's lane text, copied exactly (holds and breaths included) | the first time a chorus comes back |
| **Counter-line** | its own short lyric, long notes in the lead's gaps | a few words ("stay with me / till the light") | a final chorus, with more reverb and delay than the lead |

Give each its own track: a lane belongs to one (section, track), and the
harmony and the counter-line sing different words.

## Writing a harmony

1. **Below, not above.** A third *below* the lead (diatonic) keeps the
   harmony out of the register where it competes with the lead and out of
   the top of the backing voice's range. A third above is brighter but reads
   as a second lead, and it climbs fast (a lead that touches B♭4 puts it on
   D5).
2. **Check every note against the chord under it**, not only against the
   lead. A strict diatonic third lands on a clash at a dominant: over A7 in D
   minor a third below D is B♭, a ♭9. Hold a chord tone there instead (A3
   under the whole melisma).
3. **No hollow intervals, no leaps the lead does not make.** When the lead
   steps (B♭ → G → F) the harmony steps with it (G → E → D). A harmony that
   jumps a fourth while the lead moves a third leaves a bare fourth between
   them on that syllable, and the singer scoops into the jump — the render
   shows it as a passing semitone clash.
4. **Oblique motion in two or three places.** Hold one note while the lead
   moves (on a held word, or across a line end). It is what makes a second
   singer audible as a person rather than a pitch-shifted copy.
5. **Loosen it.** Same onsets and lengths as the lead read as fake:
   - onsets 15–60 ms late (a singer following), a few 5–25 ms early;
   - note lengths scaled independently (0.8–1.1×), and phrase ends released
     at different moments from the lead (earlier on some lines, later on
     one);
   - its own velocities, never copied from the lead, and **not its loudest
     note on the lead's peak** — that one word jumps out;
   - a slow pitch-bend drift of a few cents (±9, mostly slightly flat) with
     `automation_set_lane {vocal: {track_id, curve: "pitch_bend"}}`;
   - its own tension curve: a swell per line peaking at a different point
     than the lead's, its ceiling below the lead's.

Keep the generator script for this in the song's `-src` folder: the lead's
notes in, the harmony's notes out, so a change to the lead re-derives it.

## Writing a counter-line

- Long notes (2–4.5 beats) placed in the lead's rests and on its held words,
  short pickups between them. Words that answer the lead, not repeat it.
- Chord tones on the long notes; the line can move while the lead holds.
- Shape each held note with tension: a swell into the middle of the note, an
  ease before the next word, and a build line by line. The last note can
  fall away into the delay tail.

## One lane, several placements

A section that plays twice sings the same take twice — a lane renders once,
from the clip `vocal_generate` returned (its first placement), and every
placement plays that audio. The other placements' clips are not read, and
emptying the first clip makes the lane refuse to render. So when a backing
part should sing in only one of two placements:

- keep the notes, and silence the track elsewhere with a **volume** lane:
  `automation_set_lane {track_id, control: "volume", points: [{bar: 1, value:
  "-inf", curve: "stepped"}, {bar: <entry>, value: <level>, curve:
  "stepped"}]}`. It survives every re-render.
- Verify the silent placement with `meter_measure {target: {track_id},
  range}`: it should read `lufs_integrated: null`. A **mute** lane did not
  silence a track-target measurement when this was written; the volume lane
  did.

## Choosing the backing voice

- Pick the backing singer by range first: the part's span inside the
  voice's `comfortable_range`. A male voice under a female lead is a strong
  colour; move the line down an octave when it sits above the voice.
- **Breathiness has no control** (no installed bank takes the curve). For a
  backing voice that should not sound airy, choose a plain or strong speaker
  (standard, rock, solid — not gentle, mellow, tired, fragile, phantom) and
  keep its tension at 0.5 or above; low tension is what sounds breathy.
- **Switching bank or speaker moves the level by a lot** — 10 dB or more
  between a soft tenor and a rock baritone singing the same notes. After a
  switch, render a stem and re-set the level before anything else.

## Level and space

- About **6 LU under the lead**, measured over the section the part sings
  in (`render_stems` with a `range`, then compare integrated loudness, or
  `meter_measure` per track over that range).
- Pan it off the centre a little (±0.2–0.25), opposite sides when there are
  two.
- More reverb than the lead (send 5–8 dB hotter) pushes it back; a
  dotted-eighth or dotted-quarter delay on a counter-line makes it trail.
- Score it with the intelligibility script too: a backing part at 85–100 % is fine
  under a lead; the missed words show which line to look at.

## Checking what was sung

When the user says a passage "does not sound right", find out whether the
render sang the written notes before rewriting them: pitch-track the stem
over that passage (librosa pyin, run with `uv run`) and print the note
every 1/8 beat for the lead and the backing part side by side. Written notes
coming out as written means the problem is the writing — the interval
between the voices, a leap, a velocity — not the synthesis.
