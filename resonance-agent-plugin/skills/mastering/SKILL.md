---
name: mastering
description: Put a finished mix onto the master bus and bring it to delivery loudness in a running resonance DAW, verifying every stage by measurement. Use when asked to master a song, make it louder, hit a streaming target, add a limiter, or prepare a track for release.
when_to_use: >-
  Triggers on requests like "master this", "make it louder", "get it to -14
  LUFS", "prepare for Spotify", "add a limiter", "it's too quiet compared to
  other tracks", "final polish".
allowed-tools: mcp__resonance__control_hello mcp__resonance__song_summary mcp__resonance__meter_measure mcp__resonance__master_summary mcp__resonance__master_plugin_params
---

# Mastering in resonance

Mastering is the last 3 dB, not a rescue. It works on the summed mix and cannot
fix a balance problem — it magnifies one. If the mix is not finished, use the
`mixing` skill first and come back.

## 0. Preflight

Call `mcp__resonance__control_hello`. This procedure needs `meter.measure`,
`master.summary`, `master.add_effect`, `master.plugin_params` and
`master.set_plugin_param`. If any is missing, the running app is older than this
plugin — say so and stop.

## 1. Establish the starting point

```
mcp__resonance__master_summary       # is there already a chain?
mcp__resonance__meter_measure        # target: master, source: "render"
```

`source: "render"` needs a **stopped transport** and no bounce in flight. Check
`soloed_track_ids` — if it is non-empty the numbers describe soloed tracks, not
the mix; clear solo and measure again.

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

## 2. Insert the chain

```
mcp__resonance__master_add_effect    # plugin_id: "com.resonance.mastering"
```

One instance. Each call *appends*, so calling it twice gives the master two
mastering plugins.

`com.resonance.mastering` is a full chain in one plugin, with its stages in a
fixed signal order:

```
corrective EQ → glue comp → saturator → tonal EQ → multiband → imager → limiter → dither
```

You do not reorder these — you enable the ones the measurement asked for. That
also means `master_move_effect` is rarely what you want here.

**Every stage defaults to OFF.** An unconfigured mastering plugin measures
identically to an empty master. Adding it is half the job; the other half is
`mcp__resonance__master_set_plugin_param`.

Read `mcp__resonance__master_plugin_params` for the real parameter names, ids,
ranges and current values before setting anything. `param` takes a parameter's
display **name** (case-insensitive), its numeric `id` from that listing, or —
on this first-party plugin — its stable string **key**, which is unambiguous
where display names repeat across stages. The stage switches are `glue_on`,
`sat_on`, `mb_on`, `img_on`, `lim_on` and `dith_on`, and the limiter's
controls are `lim_ceiling` (dBTP) and `lim_release` (ms), but confirm ranges
and current values against `master_plugin_params` rather than trusting this
paragraph: it is a map, not the territory.

## 3. Enable stages one at a time, measuring after each

Enable, set, measure, compare. One stage per pass, always. A chain built without
measuring between stages cannot be debugged afterwards — you will not know which
stage cost you the transients.

Reach for a stage only when a number asked for it:

| Measurement | Stage |
|---|---|
| A narrow resonance or rumble visible as a lopsided `bands` share | corrective EQ |
| The mix does not cohere; parts sit separately | glue compressor, gently |
| Sterile, wants density | saturator |
| Broad tonal tilt across `bands` | tonal EQ |
| One band moves independently of the rest | multiband |
| `correlation` too high (narrow) or `mono_penalty_db` costly | imager |
| Needs level and `true_peak_db` is the constraint | limiter |

The limiter is the only stage that reliably buys loudness. Raising
`master_set_volume` does not: it lifts the loudest transient along with
everything else, so the peak hits full scale long before the average catches up.
Equally, **do not chase loudness by pulling the drums down** — that trades a
level problem for a balance problem, and the balance problem is the one
listeners hear.

## 4. Hit the targets

| Target | Value | Why |
|---|---|---|
| True peak | **≤ -1 dBTP**, -2 dBTP safer | Spotify's recommendation; lossy codecs push peaks up |
| Integrated | about **-14 LUFS** is the ceiling of usefulness | Platforms normalise: louder gets turned back down and you keep only the squashed dynamics |
| Crest | keep above ~8 dB | Below that is squashed |

Watch `crest_db` fall as you push the limiter. When more level starts costing
crest faster than it gains loudness, you have found the stopping point. That
tradeoff is the whole craft; there is no number that announces it.

## 5. Dither last, and only if you are reducing bit depth

"Dither On" belongs at the very end of the chain and only when the render target
is a lower bit depth than the session. Dithering a 24-bit deliverable adds noise
for nothing.

## 6. Verify and deliver

Final `mcp__resonance__meter_measure` on the master with `source: "render"`,
transport stopped. Confirm integrated, true peak, crest and `clipped_samples: 0`
against the table above, and report the before/after pair for each.

Then `mcp__resonance__render_mixdown` for the user to actually listen to. It
always bounces the whole song, and offset 0 in the file is the earliest clip,
not bar 1 — it is a deliverable, never a measuring instrument. It refuses to
overwrite until you pass `overwrite: true`, so ask before setting that.

Report the chain you built as an ordered list of *stage → the measurement that
justified it → what it cost*. Every step is undoable via `edit_undo` and visible
live in the GUI's master strip.

## Additional resources

- `${CLAUDE_PLUGIN_ROOT}/skills/mixing/references/reading-meters.md` — full meter
  field semantics, the `null`-vs-zero and `render`-vs-`live` traps, and the
  per-stage loudness targets in one table. Shared with the `mixing` skill.
