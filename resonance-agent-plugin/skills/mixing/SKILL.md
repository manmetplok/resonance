---
name: mixing
description: Balance and tonally shape a song in a running resonance DAW by measuring it, not by guessing. Use when asked to mix, balance, fix levels, sort out the low end, widen or narrow the stereo image, or diagnose why a mix sounds muddy, thin, boxy or lopsided.
when_to_use: >-
  Triggers on requests like "mix this", "balance the tracks", "the vocal is
  buried", "too much low end", "why does this sound muddy", "check the mix",
  "the drums are too loud".
allowed-tools: mcp__resonance__control_hello mcp__resonance__song_summary mcp__resonance__song_tracks mcp__resonance__song_sections mcp__resonance__meter_stems mcp__resonance__meter_measure mcp__resonance__master_summary mcp__resonance__track_plugin_params mcp__resonance__automation_lanes mcp__resonance__automation_set_lane
---

# Mixing in resonance

You cannot hear the song. `meter_stems` and `meter_measure` are your ears, and
they are good ones — use them the way an engineer uses monitors: constantly, and
before every decision.

This procedure works on any project. Nothing below assumes a track name, a track
count, or a genre. Read the song, then decide.

## 0. Preflight

Call `mcp__resonance__control_hello` first. It reports the running app's version
and `capabilities`. This procedure needs `meter.stems`, `meter.measure` and
`mixer.set_volume_db`. If any is missing, the app binary is older than this
plugin: say so and stop, rather than working around it. Nothing else here can
substitute for a measurement.

Then `mcp__resonance__song_summary` and `mcp__resonance__song_tracks` for the
track list and ids. If `edit_status` shows unsaved work you did not make, ask
before touching anything.

## 1. Measure the whole song, once

```
mcp__resonance__meter_stems   # whole song, include_busses: false
```

One pass, one shared range, every track plus the master — all directly
comparable. Costs one full-length render per track, so a large project may
exceed the tool's wait and return a running job; poll `job_status` or block on
`job_wait`.

Before reading anything: check `soloed_track_ids` on the master entry. If it is
non-empty, the master numbers describe only the soloed tracks and are useless as
a picture of the mix. Clear solo (`mixer_set_solo`) and measure again. **Never
solo as a measurement technique** — a track target already ignores solo.

See `${CLAUDE_SKILL_DIR}/references/reading-meters.md` for what each field means
and the numbers worth aiming at. Read it before your first diagnosis; the field
semantics are not guessable, and several of them are `null` rather than zero
when they do not apply.

## 2. Diagnose in a fixed order

Work down this list. Each level's problems masquerade as the next level's, so
fixing out of order means fixing the same thing twice.

1. **Faults** — `clipped_samples > 0` anywhere, `correlation` negative on the
   master, a track measuring near-silence that should not be. These are bugs in
   the mix, not taste. Fix them before anything else.
2. **Balance** — differences in `lufs_integrated` between tracks. This is where
   most of the perceived problem in a bad mix actually lives.
3. **Tone** — `bands` shares, compared between tracks and against the master.
4. **Dynamics and width** — `crest_db`, `lra`, `mono_penalty_db`.

State the diagnosis in plain language with the number that supports it before
you change anything. "The lead is 9 LU under the drums" is a diagnosis; "the
vocal needs more presence" is not.

## 3. Change one class of thing per pass

In this order, and only as far down as the problem requires:

| Problem | Reach for |
|---|---|
| Balance | `mcp__resonance__mixer_set_volume_db` — prefer it over `mixer_set_volume`; deltas in LU map 1:1 onto dB |
| Placement | `mcp__resonance__mixer_set_pan` |
| Two things fighting for the same range | `mcp__resonance__track_add_effect` with `com.resonance.eq`, then `mcp__resonance__track_set_plugin_param` |
| A group needing one move | `mcp__resonance__bus_create` + `mcp__resonance__track_set_output` |
| Shared ambience | `mcp__resonance__track_add_send` into a return bus |
| A part that will not sit still | `com.resonance.compressor` on the track |
| One part ducking under another | `mcp__resonance__track_set_sidechain` |
| A fader move that has to happen over time (a fade, a level ride) | `mcp__resonance__automation_set_lane`, read back with `mcp__resonance__automation_lanes` |

Rules that hold regardless of the song:

- **Balance with track faders, never the master fader.** The master is a single
  scalar after everything; it cannot change any relationship, and it cannot make
  the mix louder without clipping the loudest transient first.
- **A plugin does nothing until it is configured.** `track_add_effect` leaves it
  at defaults. Read `track_plugin_params` for the real ids and ranges, then set
  them. Never guess a parameter id.
- **Do not pull everything down to make room.** Move the outlier, not the mix.
- Leave headroom. A finished mix belongs around **-23..-18 LUFS integrated**
  with peaks well under 0 dBFS. -14 LUFS is a *mastering* target; hitting it
  here means squashing the mix and is the single most common way to ruin one.
  Loudness is the mastering skill's job.

## 4. Verify, then decide whether to continue

Re-measure over **the same range** as step 1 and compare the specific number you
set out to move. An edit you have not measured has not happened.

Two failure modes to watch for in yourself:

- **Drift.** If a change did not move the number you predicted, you do not
  understand the signal path yet. Read `song_tracks` for routing and
  `track_plugin_params` for what is actually on the track before trying again.
- **Fiddling.** Once the faults are gone, the balance is deliberate and the tone
  is even, stop and hand it back. There is no measurement that says "finished",
  so the stopping rule has to be yours.

Report what you changed as a short list of *track → old → new, and why*, so the
user can undo any single decision. Every edit went through the app's normal undo
path and is visible live in the GUI; `edit_undo` reverses them one at a time.

## Additional resources

- `${CLAUDE_SKILL_DIR}/references/reading-meters.md` — what every meter field
  means, the numbers worth aiming at, and the traps (`null` vs zero, `live` vs
  `render`, solo-sensitivity, band shares being relative).
