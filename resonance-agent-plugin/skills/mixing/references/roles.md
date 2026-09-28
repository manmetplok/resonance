# Roles: per-role staging

Shared by the `mixing` and `spatial` skills. A starting point per musical
*role*, never per track name or genre. Every number is a place to start and
then verify with a meter, not a setting to copy.

## 1. Infer each track's role from what the app reports

Read `song_tracks` (and `song_summary`), then decide each track's role from
data, in this order. Never from the track's name: a name is the user's label,
and the same word means different parts in different songs.

| Evidence | Where it comes from | Points to |
|---|---|---|
| `kind` is `drums`, or `parent_id` points at a drum kit | `song_tracks` | drums. A kit's sub-tracks are its outputs; tell kick from snare from cymbals by their measured `bands` (kick: `low` dominant; snare: `mid`; cymbals and overheads: `high` and `air`). `meter_stems` folds sub-tracks into the kit's entry, so measure each one on its own with `meter_measure {target: {track_id: <sub-track>}}` |
| `kind` is `vocal` | `song_tracks` | a vocal. With several, the lead is the one singing through most sections (`song_vocal`, `song_sections`); the rest are backing |
| Notes mostly below about MIDI 52 (E3), one at a time | `song_notes` | bass |
| Long notes held across chords, several at once | `song_notes` | pad (or sustained keys) |
| Short chord hits, several notes at once | `song_notes` | keys or rhythm part |
| One note at a time, above the vocal's range, carrying a line | `song_notes` | lead synth or lead instrument |
| Sparse, irregular clips; noise or effect sounds | `song_tracks` clip layout, `bands` | FX / texture |
| An `audio` track: no notes to read | `meter_stems` `bands`, `correlation`, `crest_db` | a strong `low` share with correlation near +1 is bass-like; otherwise ask |

When the evidence is ambiguous — an audio take, two candidates for the lead —
**ask the user**, naming the evidence ("track 7 is an audio take with most of its
energy in the mids; is it a rhythm guitar?"). A wrong role puts the part in the
wrong layer, and depth is only as good as the layer assignment.

A genre is a **parameter** the user gives, not a branch: it moves a number
inside the ranges below (drier drums, wider backing), never which procedure you
follow.

## 2. The staging table

Level is relative to the drums in LU, measured by `meter_stems` (the tripwire
table in reading-meters.md). Pan is `mixer_set_pan` (-1 left .. +1 right).
Width is what the role wants on the stereo meter (`side_mid_db` above 150 Hz);
how to get it is the `spatial` skill's width.md. Layer is the depth layer
(spatial's depth.md).

**Room send** is the track's post-fader send into the shared room return, as a
*first guess* `level_db`: it only has to be close enough for the first depth
pass to measure the return. The send you keep comes from the layer's DRR
target, because the estimate is plain arithmetic on it:

  DRR = −(send + `return_gain_db`)

Read `return_gain_db` from that pass (`meter_stems` `detail: ["depth"]`, the
track's `sends` entry for the room), then set the send to
−(target DRR) − `return_gain_db`. With a return gain of −6 dB, a front part
aiming at +12 wants a send of −6, and a back part aiming at −2 wants +8. When
that lands far from the first guess, trust the arithmetic, not the table. The
targets are in depth.md (front +10 or more, middle +3 to +8, back 0 or less).
Moving the return's own fader moves `return_gain_db`, and so every sender's
DRR, by the same amount.

**Pre-delay** is a setting of a *return*, not of a send, and one shared room
has one pre-delay. The column says what each role wants from the room it is
on. Set the shared room's pre-delay for the front layer (the lead's 20-40 ms);
the roles that want 0-10 ms (pads, FX) are what the second, back return in
depth.md is for.

| Role | Layer | Level (LU) | Pan | Width | Room send (first guess) | Pre-delay wanted | Character |
|---|---|---|---|---|---|---|---|
| Kick | front | 0 (drums) | centre | mono | none, or ≤ -24 | — | drum bus |
| Snare | front | 0 (drums) | centre | narrow | -18 to -12 | 10-20 ms | drum bus |
| Drum overheads / room | middle | with the kit | as recorded, or ±0.6-1.0 for a stereo pair | natural | -18 to -12 | — | drum bus |
| Bass | front | -2 | centre | mono below 150 Hz | none | — | track, even-dominant, low-weighted |
| Lead vocal | front | -4 | centre | mono; width only from its reverb and doubles | -20 to -14, ducked | 20-40 ms (tempo) | track, even-dominant |
| Backing vocals | middle | -8 to -10 | pairs at ±0.3-0.7 | moderate | -14 to -10 | 10-20 ms | vocal bus |
| Rhythm guitars / doubled parts | middle | -6 | doubles ±0.8-1.0; a single part ±0.3-0.5 | from the doubles | -16 to -12 | 10-20 ms | music bus |
| Keys | middle | -8 | ±0.2-0.5 | moderate | -14 to -10 | 10-20 ms | music bus |
| Lead synth / lead instrument | front or middle | -4 to -6 | centre or ±0.2 | narrow to moderate | -16 to -12 | 20-30 ms | track |
| Pads | back | -13 | centre (stereo source) | wide | -10 to -6 | 0-10 ms | music bus, darker |
| FX / texture | back | -11 to -18 | anywhere, moving | wide | -8 to -4 | 0 ms | — |

Rules that hold for every role:

- **Low end is mono.** Everything below about 150 Hz sits in the centre:
  kick, bass, and the lows of everything else (spatial's width.md).
- **Front means dry, bright and centred; back means wet, darker and wide.**
  The layers are a contrast, not absolute settings: verify the *ordering* with
  `meter_stems` `detail: ["depth"]` (lead's `drr_db_estimate` above backing's,
  backing's above the pads').
- **A hard pan costs about 3 dB.** The pan law is stereo balance, so moving a
  mono part from centre to ±1.0 drops its loudness about 3 dB (about 2 dB at
  ±0.5). Re-measure and give it back on the fader (spatial's width.md has the
  table).
- **The table is a start.** If `meter_stems` says a role sits 12 LU off its
  row, ask whether that was deliberate before moving it.
