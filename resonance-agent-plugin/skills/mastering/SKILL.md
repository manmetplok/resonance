---
name: mastering
description: Put a finished mix onto the master bus and bring it to delivery loudness in a running resonance DAW, verifying every stage by measurement. Use when asked to master a song, make it louder, hit a streaming target, add a limiter, match a reference track, add master-bus warmth or width, or prepare a track for release.
when_to_use: >-
  Triggers on requests like "master this", "make it louder", "get it to -14
  LUFS", "prepare for Spotify", "add a limiter", "it's too quiet compared to
  other tracks", "make it sound like this reference", "final polish", "it
  sounds sterile".
allowed-tools: mcp__resonance__control_hello mcp__resonance__song_summary mcp__resonance__meter_measure mcp__resonance__meter_snapshot mcp__resonance__meter_compare mcp__resonance__meter_probe mcp__resonance__master_summary mcp__resonance__master_plugin_params
---

# Mastering in resonance

Mastering is the last 3 dB, not a rescue. It works on the summed mix and cannot
fix a balance problem — it magnifies one. If the mix is not finished, use the
`mixing` skill (and `spatial` for width and depth) first and come back.

## 0. Preflight

Call `mcp__resonance__control_hello`. This procedure needs `meter.measure`,
`meter.snapshot`, `meter.compare`, `meter.probe`, `master.summary`,
`master.add_effect`, `master.plugin_params` and `master.set_plugin_param`. If
any is missing, the running app is older than this plugin — say so and stop.

## 1. Establish the starting point

```
mcp__resonance__master_summary       # is there already a chain?
mcp__resonance__meter_snapshot       # target: master, all details — the baseline
```

A measurement needs a **stopped transport** and no bounce in flight. Check
`soloed_track_ids` — if it is non-empty the numbers describe soloed tracks, not
the mix; clear solo and measure again. Keep the `snapshot_id`: every stage
below is judged against it with `meter_compare`, which gain-matches the two
sides so a louder master does not read as a better one.

An empty `plugins` array on the master means the mix is going out completely
unprocessed. That is the normal starting state, not a problem.

**Gate on the mix before proceeding.** A finished mix sits around
**-23..-18 LUFS integrated** with peaks well under 0 dBFS and `clipped_samples`
at 0. If it does not:

| What you measure | What it means | Do |
|---|---|---|
| `clipped_samples > 0` | The mix is already clipping | Stop. Fix in the mix — mastering cannot remove it. |
| Above -14 LUFS | Already at mastering loudness; no headroom left | Stop. There is nothing to master into. |
| Below about -26 LUFS | Unusually quiet | Check for a stray master fader move before assuming it needs 12 dB of limiting. |
| `correlation` negative | Anti-phase content | Fix in the mix. A limiter will make it worse. |
| Low-band stereo correlation under +0.9 | Wide lows | Fix in the mix (`spatial`); the imager's side high-pass is the last resort. |

## 2. Insert the chain

```
mcp__resonance__master_add_effect    # plugin_id: "com.resonance.mastering"
```

One instance. Each call *appends*, so calling it twice gives the master two
mastering plugins.

`com.resonance.mastering` is a full chain in one plugin, with its stages in a
fixed signal order:

```
input trim → corrective EQ → glue comp → saturator → tonal EQ → multiband → imager → clipper → limiter → dither
```

You do not reorder these — you enable the ones the measurement asked for. That
also means `master_move_effect` is rarely what you want here.

**Every stage defaults to OFF.** An unconfigured mastering plugin measures
identically to an empty master. Adding it is half the job; the other half is
`mcp__resonance__master_set_plugin_param`, which takes the stable string
**key** below (or a display name or numeric id from `master_plugin_params`).
Keys are unambiguous where display names repeat across stages. Confirm ranges
and current values against `master_plugin_params` before setting anything: this
map names the levers, the listing is the territory.

<!-- keys: com.resonance.mastering -->
| Stage | Switch | Controls |
|---|---|---|
| Input | — | `input_trim_db` (dB into the chain: the loudness lever) |
| Corrective EQ | `corr_b{n}_on` | per band n = 0-3: `corr_b{n}_type` (0 Bell, 1 low shelf, 2 high shelf, 3 high-pass, 4 low-pass), `corr_b{n}_freq`, `corr_b{n}_q`, `corr_b{n}_gain`, `corr_b{n}_ms` (`Stereo` / `Mid` / `Side`). Defaults: b0 high-pass 30 Hz, b1 250 Hz, b2 500 Hz, b3 3 kHz |
| Glue compressor | `glue_on` | `glue_threshold`, `glue_ratio`, `glue_attack`, `glue_release`, `glue_knee`, `glue_makeup`, `glue_mix` |
| Saturator | `sat_on` | `sat_mode` (`Blend`, `Tube`, `Tape`, `Transformer`, `Console`, `Warm`, `Inflator`), `sat_drive` (dB), `sat_mix`; `Blend` only: `sat_character` (0 Tube … 1 Tape), `sat_shaper` (0 smooth, 1 gritty); `Inflator` only: `sat_curve` |
| Tonal EQ | `tone_b{n}_on` | as the corrective EQ, with the tone prefix. Defaults: b0 low shelf 100 Hz, b1 700 Hz, b2 2.5 kHz, b3 high shelf 10 kHz |
| Multiband | `mb_on` | `mb_xo1`, `mb_xo2`, `mb_xo3` (crossovers); per band n = 0-3: `mb_b{n}_on`, `mb_b{n}_thresh`, `mb_b{n}_ratio`, `mb_b{n}_attack`, `mb_b{n}_release`, `mb_b{n}_knee`, `mb_b{n}_mix`, `mb_b{n}_gain` |
| Imager | `img_on` | `img_width` (0-2, 1 = unchanged), `img_side_hpf_on`, `img_side_hpf_freq` (the mono-maker), `img_b{n}_width` (per multiband band, low first; needs the crossovers) |
| Clipper | `clip_on` | `clip_drive` (dB of peak shaved), `clip_shape` (0 hard … 1 soft) |
| Limiter | `lim_on` | `lim_ceiling` (dBTP), `lim_release` (ms) |
| Dither | `dith_on` | `dith_bits`, `dith_ns` (noise shaping) |
<!-- /keys -->

A de-harsh stage (a resonance suppressor, band-limited to the presence range)
is being added to this chain. TODO(de-harsh): name its switch and controls here
once it lands; until then, de-harsh in the mix (`mixing` skill, character.md).

## 3. Enable stages one at a time, measuring after each

Enable, set, compare. One stage per pass, always: `meter_compare {a:
snapshot_id}` after each, reading the deltas at matched loudness. A chain built
without measuring between stages cannot be debugged afterwards — you will not
know which stage cost you the transients.

Reach for a stage only when a number asked for it:

| Measurement | Stage |
|---|---|
| A narrow resonance in `peaks`, or rumble below 30 Hz | corrective EQ |
| A presence peak (2-5 kHz `peaks`, high `presence_peakiness_db`) | corrective EQ bell, or the de-harsh stage once it exists |
| The mix does not cohere; parts sit separately | glue compressor, gently |
| Sterile (the test below) | saturator |
| `tilt_db_per_oct` off the target slope | tonal EQ |
| One band moves independently of the rest | multiband |
| Narrow highs, or wide lows (stereo detail) | imager |
| The limiter pumps, or `crest_db` is high with sharp transients | clipper, 1-3 dB |
| Needs level and `true_peak_db` is the constraint | limiter |

### Sterile: a test, not an adjective

The mix is sterile when **both** hold:

- `tilt_db_per_oct` is flatter than about -4.5 (from the baseline's spectrum
  detail), **and**
- `meter_probe` on the master, and on the mix busses if the mix has any, shows
  `thd_pct` under 0.1 %: nothing in the path adds harmonics.

If the mix busses already carry colour and the probe shows it, the mix is not
sterile; do not stack a master stage on top of it. Otherwise, the saturator:

<!-- keys: com.resonance.mastering -->
| Key | Setting |
|---|---|
| `sat_on` | `On` |
| `sat_mode` | `Tape` or `Transformer` (even-leaning, low-weighted); `Tube` or `Warm` for more H2 |
| `sat_mix` | 0.1-0.3 for parallel |
| `sat_drive` | raised until the probe hits the master THD band |
| `Inflator` | density (loudness without more limiting), odd-only: not a warmth tool |
<!-- /keys -->

Set the drive with `meter_probe {level_dbfs: <the master's true peak before the
chain>}`: `thd_pct` 0.1-1 %, `h2_h3_db` above 0 and `decay_db_per_order` 6 or
more. The full warmth procedure, with its stop rules, is the mixing skill's
`${CLAUDE_PLUGIN_ROOT}/skills/mixing/references/character.md`.

### Tone: tilt toward the target slope

Commercial masters average a `tilt_db_per_oct` of about -4.5 to -5. Move toward
it with the tonal EQ's shelves, ±0.5-1.5 dB per move, and compare after each:

<!-- keys: com.resonance.mastering -->
| Move | Keys |
|---|---|
| Low weight | `tone_b0_on`, `tone_b0_gain` (low shelf, 100 Hz) |
| Top | `tone_b3_on`, `tone_b3_gain` (high shelf, 10 kHz) |
| Air or width at the top, mono sum untouched | a band's `tone_b{n}_ms` `Side` |
| Side-only low cut | a high-pass band's `corr_b{n}_ms` `Side` |
<!-- /keys -->

### Width

<!-- keys: com.resonance.mastering -->
| Move | Keys |
|---|---|
| Mono lows | `img_on`, `img_side_hpf_on`, `img_side_hpf_freq` 100-150 Hz |
| Wider highs | `img_width` or the per-band `img_b{n}_width` (low band first), 1.05-1.2, above about 200-300 Hz only |
<!-- /keys -->

Side gain alone vanishes in mono, so keep the moves small. Verify with the
compare's stereo deltas: the highs' `side_mid_db` up, the low bands'
`correlation` and `mono_loss_db` no worse.

### Clipper, then limiter

<!-- keys: com.resonance.mastering -->
The clipper shaves transients so the limiter does less: `clip_on` `On`,
`clip_drive` 1-3 dB, `clip_shape` toward soft. It lowers the peaks by
`clip_drive` and adds no loudness itself, so raise `input_trim_db` by about the
same amount to use the headroom it made. Then `lim_on` `On`, `lim_ceiling` -1
(-2 safer through lossy codecs), `lim_release` about 50 ms, and bring the level
up with `input_trim_db`: about -14 minus the pre-master integrated LUFS.
<!-- /keys -->

The limiter is the only stage that reliably buys loudness. Raising
`master_set_volume` does not: it lifts the loudest transient along with
everything else, so the peak hits full scale long before the average catches up.
Equally, **do not chase loudness by pulling the drums down** — that trades a
level problem for a balance problem, and the balance problem is the one
listeners hear.

## 3b. A reference track (optional)

When the user supplies a commercial reference, compare against it instead of
against generic targets. The reference must be in the project pool
(`pool_import`, then `pool_list` for its id).

> TODO(master.assist): the `master.assist` control method and its MCP tool are
> being built. Resolve this branch against the landed tool name and result
> shape before shipping; until then, skip 3b if `control_hello` does not list
> `master.assist`.

With `master.assist` in `capabilities`:

1. Call it in reference mode with the reference's pool asset id (or
   `{mode: "genre", genre: <what the user named>}` for the built-in genre
   target bands). It **suggests, it does not apply**: each suggestion comes with
   its rationale.
2. Treat each suggestion as a measurement asking for a stage (the table in 3).
   Apply the ones you agree with through `master_set_plugin_param`, one stage
   per pass, with `meter_compare` after each, as above.
3. Report which suggestions you took, which you did not, and why.

A genre is a parameter the user gives, never a branch baked into this
procedure.

## 4. Hit the targets

| Target | Value | Why |
|---|---|---|
| True peak | **≤ -1 dBTP**, -2 dBTP safer | Spotify's recommendation; lossy codecs push peaks up |
| Integrated | about **-14 LUFS** is the ceiling of usefulness | Platforms normalise: louder gets turned back down and you keep only the squashed dynamics |
| `psr_db` | **8 or more** in the loudest section | Below that the limiter is eating the transients |
| `plr_db` | 8-12 dB | Lower means limited hard |
| Crest | keep above ~8 dB | Below that is squashed |

Watch `crest_db` and `psr_db` fall as you push the limiter. When more level
starts costing crest faster than it gains loudness, you have found the stopping
point. That tradeoff is the whole craft; there is no number that announces it.

## 5. Dither last, and only if you are reducing bit depth

<!-- keys: com.resonance.mastering -->
`dith_on` belongs at the very end of the chain and only when the delivery is a
lower bit depth than the session, e.g. a 16-bit file: `dith_bits` 16, and
`dith_ns` `On` for noise shaping. Dithering a 24-bit deliverable adds noise for
nothing.
<!-- /keys -->

## 6. Verify and deliver

Final `meter_compare {a: snapshot_id, match: "none"}` for the as-measured
before/after, and a `meter_measure` of the master with
`detail: ["spectrum", "stereo", "dynamics"]`. Confirm integrated, true peak,
crest, `psr_db` and `clipped_samples: 0` against the table above, and report the
before/after pair for each.

Then `mcp__resonance__render_mixdown` for the user to listen to. For a delivery
file, pass `platform` so the file lands exactly on the platform's target; the
project itself is untouched. Platform targets, the PLR/PSR guidance and the
dither rule are in `${CLAUDE_SKILL_DIR}/references/delivery.md`. The bounce
always covers the whole song, and offset 0 in the file is the earliest clip, not
bar 1 — it is a deliverable, never a measuring instrument. It refuses to
overwrite until you pass `overwrite: true`, so ask before setting that.

Report the chain you built as an ordered list of *stage → the measurement that
justified it → what it cost*. Every step is undoable via `edit_undo` and visible
live in the GUI's master strip.

## Additional resources

- `${CLAUDE_SKILL_DIR}/references/delivery.md` — platform loudness targets,
  PLR/PSR, the dither rule, and `render_mixdown` normalisation.
- `${CLAUDE_PLUGIN_ROOT}/skills/mixing/references/reading-meters.md` — full meter
  field semantics, the `null`-vs-zero and `render`-vs-`live` traps, the compare
  and probe results. Shared with the `mixing` skill.
- `${CLAUDE_PLUGIN_ROOT}/skills/mixing/references/character.md` — the warmth
  vocabulary, THD targets and stop rules.
