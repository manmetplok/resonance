# The resonance drum map

Ground truth: `resonance-common/src/drum_map.rs`. Thirty pads, shared by the
`resonance-drums` plugin and every generator.

**It is General MIDI in the middle and not GM at the edges.** Notes 36–61 mostly
follow GM; notes 21–31 are extended articulations that exist in no GM chart, and
note 39 is repurposed. Writing from GM memory alone produces a part that triggers
the wrong pads.

## Core kit

| Note | Pad |
|---|---|
| 36 | Kick |
| 38 | Snare |
| 37 | Rimshot *(GM: Side Stick)* |
| 42 | Hi-Hat Closed |
| 46 | Hi-Hat Open |
| 44 | Hi-Hat Pedal |
| 45 | Tom Low |
| 47 | Tom Mid |
| 50 | Tom High |
| 49 | Crash 16 Edge *(GM: Crash 1)* |
| 57 | Crash 18 Edge *(GM: Crash 2)* |
| 51 | Ride Edge *(GM: Ride 1)* |
| 53 | Ride Bell |
| 52 | China Edge *(GM: Chinese Cymbal)* |

## Departures from General MIDI

| Note | Pad | GM says |
|---|---|---|
| **39** | **Sidestick** | Hand Clap |
| 21 | Snare Flam | — |
| 22 | Snare Roll | — |
| 23 | Snare Handtuch | — |
| 24 | Hi-Hat Half Open | — |
| 25 | Hi-Hat Loose | — |
| 26 | Hi-Hat Pressed | — |
| 27 | Hi-Hat Trash Open | — |
| 28 | Crash 16 Bell | — |
| 29 | Crash 16 Tip | — |
| 31 | Count Stick | GM: Sticks |

Note the two sidestick-family sounds: **37 Rimshot** and **39 Sidestick** are
different pads. A GM-trained guess reaches for 37 when a ballad wants 39.

## Extended articulations

| Note | Pad |
|---|---|
| 55 | Crash 18 Bell |
| 58 | Crash 18 Tip |
| 59 | Ride Tip |
| 60 | China Bell |
| 61 | China Tip |

`56` (Cowbell) is GM and reachable in a drumroll, but **is not a
`resonance-drums` pad** — it will not sound on the plugin.

## Hi-hat family

Seven articulations, which is the single biggest expressive resource in the map
and the one most often left unused:

| Note | Pad | Use |
|---|---|---|
| 42 | Closed | The default eighth/sixteenth pulse |
| 26 | Pressed | Slightly open under pressure — driving rock |
| 25 | Loose | Looser still; funk sixteenths |
| 24 | Half Open | The "sizzle" before a downbeat |
| 46 | Open | Accent, usually on an upbeat, closed on the next hit |
| 44 | Pedal | Foot on the offbeat — jazz and shuffle |
| 27 | Trash Open | Wide and dirty; punk, hardcore |

A hat line that alternates 42 and 26 with the odd 46 sounds played. A hat line of
nothing but 42 sounds programmed.

## Cymbal articulations

Edge / Bell / Tip are three separate pads per cymbal, not velocity layers:

- **Edge** — the full crash or wash. 49, 57 (crashes), 51 (ride), 52 (china).
- **Bell** — the pinging cut-through. 53 (ride bell) is the classic; 28, 55, 60
  for crash and china bells.
- **Tip** — controlled, quiet articulation. 59 (ride tip), 29, 58, 61.

A jazz ride pattern is mostly **51 Ride Edge** with **53 Ride Bell** for accents,
not a crash.

## Snare articulations

| Note | Pad | Use |
|---|---|---|
| 38 | Snare | Backbeat, ghosts (velocity does the work) |
| 37 | Rimshot | Hard accent, cutting |
| 39 | Sidestick | Ballad verses, bossa, anywhere a full backbeat is too much |
| 21 | Snare Flam | Grace-note double; fills and marches |
| 22 | Snare Roll | Sustained roll |
| 23 | Snare Handtuch | Damped ("tea towel") — dry, thuddy |

Ghost notes are **note 38 at low velocity** (20–45), not a separate pad.

## Velocity guide

| Role | Velocity |
|---|---|
| Backbeat snare, crash | 110–127 |
| Kick | 95–115 |
| Accented hat, ride bell | 90–105 |
| Unaccented hat, ride | 60–80 |
| Ghost snare | 20–45 |
| Pedal hat | 40–60 |

Range is 1–127. Velocity 0 is not a rest — omit the note instead.
