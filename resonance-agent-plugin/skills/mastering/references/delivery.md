# Delivery: targets and the file

## Platform loudness targets

Platforms normalise playback to a target, so a master louder than the target is
turned down and keeps only its lost dynamics. Master to the target of where it
will be heard most; when in doubt, -14 LUFS and ≤ -1 dBTP.

| Platform / use | Integrated | True peak | `render_mixdown` `platform` |
|---|---|---|---|
| Spotify | -14 LUFS | -1 dBTP (-2 if the master is louder than -14) | `spotify` |
| YouTube | -14 LUFS | -1 dBTP | `youtube` |
| Tidal | -14 LUFS | -1 dBTP | `tidal` |
| Amazon Music | -14 LUFS | -2 dBTP | `amazon` |
| Deezer | -15 LUFS | -1 dBTP | `deezer` |
| Apple Music | -16 LUFS | -1 dBTP | `apple` |
| Club / DJ play | -9 to -6 LUFS (the shorthand uses -8) | -0.3 dBTP | `club` |

Reports that Spotify's normal mode changed, and the disputed -19 "quiet"
figure, do not change the -14 / -1 dBTP recommendation.

## Dynamics targets (from `detail: ["dynamics"]`)

| Measure | Target | Reading it |
|---|---|---|
| `plr_db` (true peak − integrated) | 8-12 dB | Lower is limited hard. A quiet genre or a dynamic song sits at the top of the range |
| `psr_db` (true peak − loudest 3 s) | 8 dB or more in the loudest section | The stop rule for limiting and density: under 8, the transients are gone |
| `crest_db` | above about 8 dB | Below that is squashed |
| Tonal slope (`tilt_db_per_oct`) | about -4.5 to -5 | The commercial average; a genre shifts it a little, the user's taste more |

## The file

`render_mixdown` writes the whole song. For a delivery file pass `platform`
(or `normalize: {target_lufs, ceiling_dbtp}` for a custom target, never both).
The mix is measured, gained to the target and true-peak limited at the ceiling;
the project itself is untouched. Read the result's `normalize` block:
`achieved_lufs` and `achieved_dbtp` are re-measured from the written file.

- `achieved_lufs` clearly under the target means reaching it would take more
  limiting than the ceiling allows. Do not push harder in the render: the
  master needs its own glue, clipper and limiter first, and its `psr_db` should
  stay at 8 or more.
- A master already at the target renders unchanged (the gain is about 0).
- Render with nothing soloed: a non-empty `soloed_track_ids` in the result
  means the file is not the mix.
- It refuses to overwrite until `overwrite: true`; ask before passing it.

## Dither

Dither only when the word length goes down, and only once, as the very last
stage: the mastering chain's dither stage, set to the delivery's bit depth
(the mastering skill names the keys). A 24-bit or float deliverable gets no
dither. A 16-bit file (CD, some distributors) gets dither, with noise shaping
unless the user asks otherwise. Never dither twice: if the distributor converts
to 16-bit itself, deliver 24-bit undithered.

## What to report

The deliverable's path, the platform or target, `achieved_lufs`,
`achieved_dbtp`, the master's `plr_db` and `psr_db`, and whether it is
dithered. If the achieved loudness missed the target, say so and why.
