---
name: spatial
description: Stage a mix in width and depth in a running resonance DAW — mono-safe stereo width, panning, decorrelation, a shared reverb room, pre-delay, return EQ and ducking, front-to-back layers — verifying every move by measurement. Use when asked to make a mix wider, deeper, more three-dimensional or more spacious, to sort out panning, to check mono compatibility, to set up reverb sends, or when something sounds flat, narrow, too wet, too dry or "in your face".
when_to_use: >-
  Triggers on requests like "make it wider", "more depth", "more space", "it
  sounds flat", "the vocal is too far back", "too much reverb", "set up
  reverb", "pan this", "does it work in mono", "more 3D", "push the pads back".
allowed-tools: mcp__resonance__control_hello mcp__resonance__song_summary mcp__resonance__song_tracks mcp__resonance__song_sections mcp__resonance__song_notes mcp__resonance__song_vocal mcp__resonance__meter_stems mcp__resonance__meter_measure mcp__resonance__meter_snapshot mcp__resonance__meter_compare mcp__resonance__track_plugin_params mcp__resonance__bus_plugin_params
---

# Width and depth in resonance

You cannot hear the image. The `stereo` and `depth` meter details are your
eyes on it: width is `side_mid_db` per band with correlation as the fault
detector, and depth is the *ordering* of `drr_db_estimate` across layers.

This is the pass after the `mixing` skill's balance, tone and character. Width
and depth moves change level too (a hard pan loses 3 dB, a send adds a wet
return), so a mix that is not balanced first cannot be staged.

Nothing here assumes a track name, a track count or a genre. Read the song, infer
roles, then decide.

## 0. Preflight

Call `mcp__resonance__control_hello`. This procedure needs `meter.stems`,
`meter.measure`, `meter.snapshot`, `meter.compare`, `mixer.set_pan`,
`track.add_send`, `bus.create`, `bus.add_effect`, `bus.set_plugin_param` and
`bus.set_sidechain` in `capabilities`. If one is missing, the running app is
older than this plugin: say so and stop.

`song_summary` and `song_tracks` for ids, kinds, pans, sends and busses. Note
the tempo track (`tempo_events`): pre-delay comes from it.

## 1. Assign roles and layers

Infer each track's role from data, not names:
`${CLAUDE_PLUGIN_ROOT}/skills/mixing/references/roles.md`. Then write down a
layer table before touching anything:

| Layer | Holds (typically) | Wants |
|---|---|---|
| front | lead vocal, kick, snare, bass, a lead line | dry, bright, centred, transient-intact |
| middle | rhythm parts, keys, overheads, backing vocals | some room, moderate width |
| back | pads, strings beds, FX, textures | wetter, darker, wider, softer |

Show the table to the user in one line per track. A wrong layer undoes every
later step.

## 2. Measure the baseline

```
mcp__resonance__meter_snapshot   # target master; keeps stereo + spectrum + dynamics
mcp__resonance__meter_stems      # detail: ["stereo", "depth"], include_busses: false
```

Keep the `snapshot_id`. Read the master's stereo bands (low-band correlation,
`side_mid_db` per band, `mono_loss_db`, `balance_db`, `one_sided`,
`haas_lag_ms`) and each track's `drr_db_estimate`, `layer_hint` and
`hf_tilt_db`. Field meanings:
`${CLAUDE_PLUGIN_ROOT}/skills/mixing/references/reading-meters.md`.

Diagnose in plain language with the number, before any move: "the master's
60-150 Hz band reads correlation 0.62: the lows are not mono", "the pads'
`drr_db_estimate` is above the lead's: the depth order is inverted".

## 3. Fix faults first

- Low-band correlation under +0.9, or any band with `mono_loss_db` worse than
  -6, or `correlation_windows.worst` under -0.1: a mono-safety fault. Find the
  stem whose own stereo bands show it.
- `haas_lag_ms` not `null` on a track: a static inter-channel delay that combs in
  mono. Fix it at its source.
- `balance_db` outside ±1 dB on the master: lopsided. Re-pan, do not re-level.

## 4. Width pass, one class of move at a time

In this order, stopping when the image is where it should be:

1. **Mono the lows** below 100-150 Hz on anything wide that carries lows.
2. **Pan** by role (roles.md), then **give back the ≈3 dB** a hard pan costs
   (the pan law is stereo balance, centre = unity), and re-measure.
3. **Doubles** before processing: two performances panned apart are the best
   width there is. Suggest recording or writing a double before widening a
   mono part artificially.
4. **Decorrelate** mono sources that must be wider, above about 150 Hz only,
   with `com.resonance.stereo` in its pure-side mode, which leaves the mono
   sum unchanged.
5. **Side shelf** for air or width at the top, and side cuts for mud, with the
   EQ's per-band M/S.
6. **Check mono fold** on the master.

Keys, presets, targets and the pan-compensation table:
`${CLAUDE_SKILL_DIR}/references/width.md`.

Verify each move: `meter_compare {a: snapshot_id}` — `side_mid_db` up above
150 Hz, while the low bands' `correlation` and `mono_loss_db` are no worse.

## 5. Depth pass

1. **One shared room.** A return bus (`bus_create`) with `com.resonance.reverb`
   at full wet, fed by post-fader sends (`track_add_send`). One room, many
   send levels: that is what makes the layers read as one space.
2. **Per-layer sends.** Front lowest, back highest (roles.md for starts).
3. **Pre-delay from tempo** on the room: 20-40 ms for the front, from the
   song's tempo.
4. **Return EQ** before the tank: high-pass about 600 Hz, low-pass about
   10 kHz.
5. **Duck the return** from the lead, keyed with `bus_set_sidechain`.
6. **Darken the back.** Back-layer parts get less top (a high shelf or the
   colour plugin's `tone`), front parts keep theirs.
7. **Verify the ordering.** `meter_stems` `detail: ["depth"]`: front's
   `drr_db_estimate` above middle's above back's, and `hf_tilt_db` falling
   front to back.

Keys, presets, the tempo table and the targets:
`${CLAUDE_SKILL_DIR}/references/depth.md`.

## 6. Verify, then stop

Final `meter_compare {a: snapshot_id}` on the master, and one more `meter_stems`
with `detail: ["stereo", "depth"]`. Accept only if:

- low-band correlation is still ≥ +0.9 and the master's mono loss is ≤ 3 dB;
- no new `haas_lag_ms`, no `one_sided` surprise;
- the DRR ordering is front > middle > back;
- the balance from the `mixing` skill still holds within 1-2 LU per role
  (re-measure `lufs_integrated`: sends and pans move it).

Stop when the ordering holds and mono is safe. Wider is not better past the
targets in width.md; wetter is not deeper once the ordering holds.

Report each move as *track → what changed → the number that justified it →
what it cost*. Every edit is an ordinary undo step.

## Additional resources

- `${CLAUDE_SKILL_DIR}/references/width.md` — width targets, the mono-maker,
  pan compensation, decorrelation and M/S keys, Haas risk.
- `${CLAUDE_SKILL_DIR}/references/depth.md` — layer cues, the room recipe,
  pre-delay from tempo, return EQ and ducking keys, DRR targets.
- `${CLAUDE_PLUGIN_ROOT}/skills/mixing/references/roles.md` — per-role
  staging and role inference.
- `${CLAUDE_PLUGIN_ROOT}/skills/mixing/references/reading-meters.md` — the
  `stereo` and `depth` fields.
- Master-bus width (`img_*` on the mastering chain) is the `mastering` skill's.
