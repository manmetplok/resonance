# The mix as a shape: the 3D tree

A way to look at a whole mix at once, from a user who mixes this way. Put
every part in three dimensions:

- **left–right**: its pan and its stereo width;
- **low–high**: the frequency range it fills;
- **front–back**: its level and how wet it is (loud and dry is in front,
  quiet and wet is behind).

A good mix forms a **tree**:

- a **trunk** at the bottom: narrow, mono, solid — kick, bass, the low end
  of everything;
- **branches** widening gradually through the mids, widest around
  2.5–5 kHz, where the wide parts (keys, doubles, reverb, overheads) live;
- a **canopy** at the very top that narrows and softens again;
- at every height, parts on **different layers**: the lead in front, the
  rhythm section middle, pads, backing vocals and returns behind.

It is **not dense**. Gaps between branches are wanted — they are what lets
each part breathe. A hole is a problem only where it reads as missing body
(nothing at all between bass and low mids) or missing presence.

## Measuring it

1. `meter_stems` over the whole song with `detail: ["spectrum", "stereo",
   "depth"]` — one pass gives every part's place on all three axes.
2. The master's `stereo` detail gives the outline of the tree: one
   `side_mid_db` per band, from 20 Hz up. A tree reads like
   -60, -50, -15, -10, -7.5, **-5.5** (widest, 2.5–5 kHz), -7.5, -8.
   Mono up to 150 Hz, a gradual widening, a narrower top.
3. The master's `third_octave` spectrum shows holes: compare each band with
   the line between its neighbours, not with a target curve.
4. Per part: frequency from `bands` and `third_octave`, width from its own
   per-band `side_mid_db` and `correlation`, layer from its loudness while it
   plays (`lufs_integrated` is gated) and the `depth` block.
5. Draw it for the user as a small text map — bands as rows, parts placed
   left/centre/right, the layer next to each. It makes the discussion
   concrete.

## What bends the tree, and the fix

| Symptom | Usual cause | Fix |
|---|---|---|
| Wide or anti-phase content below 150 Hz | a stereo chorus or drift on a synth; a granular or ping-pong delay; overhead or room mics carrying kick rumble | high-pass that part (cymbals around 200 Hz, a delay's wet signal around 250 Hz), or put a mono-below stage on it (Resonance Stereo, mono below ~160 Hz) |
| The width jumps from mono to full at 150 Hz (short trunk) | the reverb return's low tail; wide keys or pads reaching low | a high-pass on the reverb's wet signal (300 Hz); the master imager's low-mid band narrower (about 0.8) and its high-mid band a little wider (1.1) |
| The canopy is wider than the branches | bright stereo parts at the top: arps, hats, shakers, air EQ | a high cut on the bright stereo part (7 kHz) so its width lands in the branches; a little reverb on dry hats to move them back |
| A hole at 100–160 Hz | everything high-passed at 100–150 Hz and a pure-sub bass | lower one low cut (keys to 100 Hz) and a clean bell around 120 Hz on the bass; half-filling it is enough |
| A dip at 1–2.5 kHz next to a bump at 400–800 Hz | boxy low mids masking the presence region | a broad cut around 600 Hz and a broad lift around 1.8 kHz, both small (1.5–2 dB) |
| The lead not in front | its reverb send raised for "more reverb" | reverb pre-delay (30–35 ms) keeps the dry voice first; keep the lead the loudest part in the mids |
| A part leaning to one side | a kit or sample recorded off-centre | measure `balance_db`, correct with a balance control — check the sign by measuring after a small move |

Fix with filters, width and sends first, levels last; then measure the
tree again over the same range. Adding a new wide part (arps, a double)
is a tree change too: measure the bars it plays in.

## Level moves near the ceiling

- A track fader stops at +6 dB in `mixer_set_volume_db`. When a part needs
  more, add the gain earlier in its chain (compressor makeup), which also
  keeps its sends in proportion.
- Every level change on a loud lead or a dense part moves the master: if
  the master has a limiter, re-measure integrated loudness and PSR after
  it and give the limiter back what the change took.
