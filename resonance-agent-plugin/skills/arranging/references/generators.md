# `generate_part` options

`options` is a role-specific object. Every field is optional and any subset
works, so `{"style": "Walking"}` is a valid whole object. Enum values are
**PascalCase and case-sensitive**. An unparseable object is rejected with the
serde error naming the offending field.

The section needs chords first (`harmony_apply_progression`) or the call is
refused. Leave `chord_count`, `beats_per_chord` and `sevenths` out: the
generator always reads the section's grid as it stands, so setting any of them
is rejected.

## bass

| Field | Values | Default |
|---|---|---|
| `style` | `RootHold`, `RootPulse`, `RootFifth`, `Octave`, `Walking`, `Motif` | `RootPulse` |
| `base_note` | MIDI floor | 28 (E1) |
| `velocity` | 0..1 | 0.85 |
| `motif_mode` | `SameIntervals`, `Augmented`, `RhythmOnly`, `FirstNoteOnly` | — (style `Motif` only) |
| `motif_phrase` | `Simple`, `MirrorMelody`, `Restricted` | — (style `Motif` only) |

**`Walking` and `Motif` are the only two that produce an actual part.** The rest
are scaffolding:

- `RootPulse` (the default) is a root note on every beat. Shipping it is the
  clearest sign nobody set `style`.
- `RootHold` — one held note per chord. Useful under a busy arrangement, or as a
  synth-bass drone. Legitimate, but a choice, not a default.
- `RootFifth` / `Octave` — root-fifth and root-octave alternation. Fine for
  driving eighth-note rock and country; monotonous over anything slower.
- `Walking` — scale-stepping line approaching the next chord root. **Needs the
  section to have a scale** (`section_set_scale`) or it has nothing to step
  through. This is the jazz option and it is genuinely good on ii-V-I motion.
- `Motif` — develops the section's shared motif, so bass and lead relate to each
  other instead of coexisting. The most musical choice when the song has a motif
  to develop.

## lead

| Field | Values | Default |
|---|---|---|
| `style` | `ArpUp`, `ArpDown`, `ArpUpDown`, `Motif` | `ArpUp` |
| `register` | `[low, high]` MIDI | `[67, 88]` |
| `note_value_ticks` | 480 = quarters, 240 = 8ths, 120 = 16ths | 240 (arp styles only) |
| `rest_density` | 0..1 | 0 (arp styles only) |
| `velocity` | 0..1 | 0.8 |
| `fill_vocal_gaps` | bool — sound only where the vocal lane is silent | false |
| `complexity` | 0..1 | 0.5 (`Motif` only) |
| `articulation` | 0 legato .. 1 staccato | 0.3 (`Motif` only) |
| `contour` | `Auto`, `Arch`, `Descending`, `Ascending`, `Wave` | `Auto` (`Motif` only) |
| `phrase_len` | 2, 4, 8 | 4 (`Motif` only) |
| `motif_len` | 0 = auto | 0 (`Motif` only) |
| `leap_chance` | 0..1 | 0.21 (`Motif` only) |
| `embellishment` | `Auto`, `Folk`, `PopBallad`, `Jazz` | `Auto` (`Motif` only) |

The arp styles are **figuration, not melody** — they spell the chord out in
order. They work as an ostinato, a synth arpeggio, or a texture behind something
else. They do not work as the thing the listener is supposed to follow.

`Motif` is real melodic development, with phrasing and contour. If the request
was "write a melody", this is the only option that answers it. The knobs worth
touching first:

- `contour` — the shape of the phrase. `Arch` is the safe, singable default;
  `Descending` reads as resolution or resignation; `Ascending` builds.
- `phrase_len` — 4 for most things, 8 for something that breathes, 2 for a hook.
- `complexity` and `leap_chance` together set how far it wanders. Raise both for
  instrumental writing, lower both for anything doubling a vocal.
- `embellishment` — `Jazz` adds the approach-note vocabulary; `Folk` and
  `PopBallad` stay diatonic.

`rest_density` above 0 is worth setting on arp styles: continuous 8ths with no
gaps is the sound of a generator, not a player.

## pad

| Field | Values | Default |
|---|---|---|
| `register` | `[low, high]` MIDI | `[52, 76]` |
| `velocity` | 0..1 | 0.7 |

**Pad has no `style`** — it always voices the chords SATB-style. The only real
decision is `register`, and it matters more than it looks: the default `[52, 76]`
sits squarely where a lead vocal and a rhythm guitar also live. Move it up or
down before adding EQ to fix the collision later.

## Choosing a role

`role` must match the track: a drum or vocal track is rejected. Use
`generate_drums` and `vocal_generate` for those.

`section_set_lane_generator` installs a generator on a (section, track) lane but
**derives no notes** — it is for pinning a lane's config and for creating the
vocal lane. To get MIDI now, call `generate_part`.

A song that sings in several sections uses **one vocal lane per section on the
same vocal track**, each addressed by passing its section as `section_id` to the
`vocal_*` tools. For a vocal lane, `options` is a partial patch in snake_case
(`{"style": "folk", "range": {"low": 55, "high": 72}}`) that merges into the
lane and keeps its lyrics and seed. To change a lane's voice or generator later,
read it with `vocal_get_params` and patch it with `vocal_set_params`; neither
renders, so follow with `vocal_generate` or `vocal_render`. No edit you make
re-renders a lane by itself: it reads `stale` in `song_vocal` (per lane, with
the track's render state the worst lane's) until one `vocal_render` after
your last edit brings it back to `rendered`. `vocal_voicebanks`
lists the installed voicebanks with their speaker ids, comfortable ranges and
which expression curves they take; a lane on a bank that is not installed
renders nothing (`voicebank_missing`), so pick one from that list. Its
`available` list holds the plok.org banks that are not installed yet:
`vocal_voicebank_install` with one of those ids downloads it (and the vocoder
it needs) as a job. Installing accepts the bank's terms for the user, so tell
them what the result says (`credit`, `terms_url`, `commercial`) before you
render with it. When you use a bank, pass its `credit` line on to the user.

## Reproducibility

`seed` makes output reproducible. Omitted, it is derived from the section id, so
repeating the same call gives the same result rather than a new random one. To
audition a different take, change the seed on purpose.

## Return value

`clip_ids`, **one per placement of the section**, in arrangement order, plus
`clip_id` as the first of them. Verify with `song_notes` on the returned id — a
generated part that came back empty or an octave out looks identical to a good
one from the tool result alone.
