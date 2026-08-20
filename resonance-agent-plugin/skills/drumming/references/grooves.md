# Groove recipes

Beat positions are **0-based and clip-relative**, in beats, matching
`notes_insert_many`'s `start_beat`. One bar of 4/4 is beats 0–3.99: count "1" is
beat 0, "2" is beat 1, "the and of 2" is beat 1.5, "4" is beat 3.

Pitches are from `drum-map.md`. Velocities in brackets.

## Swing — the arithmetic

Straight eighths sit at `.0` and `.5`. **Swung eighths put the offbeat at 2/3 of
the beat: `.667`.** That is the whole difference, and it is why the pattern
library has no jazz groove — every built-in is on a straight grid.

| Feel | Offbeat lands at |
|---|---|
| Straight | `.5` |
| Swung (triplet) | `.667` |
| Hard shuffle | `.667` with the downbeat accented harder |
| Light swing | `.58`–`.62` — between the two, common at fast tempos |

Swing lightens as tempo rises: at 260 BPM a jazz ride is nearly straight. If the
user asks for "a bit of swing", `.6` is a better answer than `.667`.

## Jazz — ride-led

The ride cymbal carries the time; the snare and kick converse under it. This is
the pattern the library cannot produce.

One bar, medium swing:

| Voice | Beats | Vel |
|---|---|---|
| Ride Edge (51) | 0, 1, 1.667, 2, 3, 3.667 | 85 / 70 / 60 / 85 / 70 / 60 |
| Hi-Hat Pedal (44) | 1, 3 | 50 |
| Snare (38) ghosts | scattered on 0.667, 2.667 | 25–35 |

The "spang-a-lang": beat, beat–offbeat, beat, beat–offbeat. Hat pedal on **2 and
4** (beats 1 and 3) is what makes it jazz rather than a swung rock beat.

Comping is deliberately irregular — snare and kick accents placed *against* the
ride, never in a repeating one-bar loop. Write two or four bars of variation, not
one bar repeated. For ballads, swap the ride for **Ride Tip (59)** and the
backbeat for **Sidestick (39)**. For up-tempo, add **Ride Bell (53)** accents on
2 and 4.

## Shuffle

Every eighth swung, both hands. Blues, Texas shuffle, `Rosanna` when the ghosts
get dense.

| Voice | Beats | Vel |
|---|---|---|
| Hi-Hat Closed (42) | 0, 0.667, 1, 1.667, 2, 2.667, 3, 3.667 | 95 / 65 alternating |
| Snare (38) | 1, 3 | 120 |
| Snare ghosts (38) | 0.667, 2.667 | 30 |
| Kick (36) | 0, 2.667 | 105 |

## Progressive — odd groupings

The DAW has a signature track, but the control API cannot write to it, so unless
the user has already placed a signature event, odd-metre material is written as
groupings inside the existing grid. The pattern's period then stops matching the
bar; note when it realigns.

| Grouping | Period (beats) | Accent beats within one period | Realigns with 4/4 every |
|---|---|---|---|
| 7/8 as 3+2+2 | 3.5 | 0, 1.5, 2.5 | 7 bars |
| 7/8 as 2+2+3 | 3.5 | 0, 1, 2 | 7 bars |
| 5/8 as 3+2 | 2.5 | 0, 1.5 | 5 bars |
| 5/4 as 3+2 | 5 | 0, 3 | 5 bars |
| 5/4 as 2+3 | 5 | 0, 2 | 5 bars |
| 9/8 as 2+2+2+3 | 4.5 | 0, 1, 2, 3 | 9 bars |
| 11/8 as 3+3+3+2 | 5.5 | 0, 1.5, 3, 4.5 | 11 bars |
| 6/8 (compound) | 3 | 0, 1.5 | 3 bars |

Put the **kick on the group heads** and the **snare on one interior accent** —
the grouping has to be audible in the kit or it is just a bar that ends early. A
crash on each realignment point tells the listener where the cycle closes.

Polymetre is the other prog device the global-meter limit actually helps with: a
5-beat kick figure over a 4-beat hat line phases naturally and resolves after 20
beats, and neither part has to fight the bar lines.

## Latin

**Son clave 3-2**, two bars, usually on Sidestick (39) or Rimshot (37):

| Bar | Beats |
|---|---|
| 1 | 0, 1.5, 3 |
| 2 | 1, 2 |

**2-3** is the same two bars swapped. **Rumba clave 3-2** moves the third stroke:
bar 1 becomes 0, 1.5, **3.5**.

**Bossa nova** — Sidestick (39) on the clave, brushed or closed hat in straight
eighths, kick on 0 and 2.5, all quiet (velocity 60–85). It is a *soft* groove;
programming it at rock velocities is the usual mistake.

**Samba** — Kick (36) on 0, 1, 2, 3 with the accent on beats 1 and 3, sixteenth
hats, and the snare pushing sixteenth syncopations. Fast, 96–108 BPM felt in 2.

## Funk

The genre where velocity does most of the work.

| Voice | Beats | Vel |
|---|---|---|
| Hi-Hat Loose (25) | every 0.25 (sixteenths) | 95 on beats, 55 between |
| Snare (38) | 1, 3 | 120 |
| Snare ghosts (38) | 0.75, 1.75, 2.25, 3.75 | 25 |
| Kick (36) | 0, 0.75, 2.5 | 110 |

**Linear funk** — no two voices on the same tick, ever. Every hit is its own
sixteenth. Strip the pattern above so nothing coincides and it becomes a
different genre.

Open-hat (46) on the last sixteenth of a bar, closed on the next downbeat, is the
standard signal that a bar is ending.

## Rock and metal

`generate_drums` covers most of this — `four-on-floor`, `halftime`, `breakbeat`,
`blast`, `industrial`. Hand-write when you need:

**Double kick** — Kick (36) on every 0.25 or 0.125, velocity 95–105 with slight
variation. Flat velocity here is what makes programmed metal sound fake.

**Half-time feel** at full tempo — snare on beat 2 only (not 1 and 3), kick
sparse, ride or china carrying eighths. Instantly halves the perceived tempo
without changing BPM.

**China accents** — China Edge (52) instead of a crash on section starts; darker
and more abrupt.

## Building a part

1. Lay the pulse (hats or ride) across the whole clip.
2. Add the backbeat.
3. Add the kick pattern.
4. **Then** vary velocity and add ghosts — this is the step that turns a grid
   into a part, and the one most often skipped.
5. Vary bar 2 (or bars 2 and 4) so the loop is not literally a loop.
6. Fill only at the seam into the next section.
