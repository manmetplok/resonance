# Depth: layers, the room, the numbers

**Depth is contrast between layers**, not an absolute setting. Verify the
*ordering* (front drier than middle, middle drier than back), never a single
track's number against a target.

## The cues, strongest first

| Cue | Front | Back | Lever here |
|---|---|---|---|
| Level | louder | quieter | the fader (the `mixing` skill's balance) |
| Direct-to-reverberant ratio (DRR) | dry | wet | the send level into the shared room |
| Pre-delay (pop/rock convention) | long, 20-40 ms: the voice lands before its room | short, 0-10 ms | the room's pre-delay, or a second return |
| Early reflections vs tail | ER carry position | tail carries the room's size | the room's ER/tail balance |
| High-frequency content | bright | darker | a high shelf or the colour plugin's output tilt on back parts; the room's wet low-pass |
| Transients | sharp | softer | less compression on front parts; softer attack on back parts |
| Width of the *source* | as recorded | narrower | a distant source reads narrower. A back-layer *bed* (pads) can still be wide: its width comes from the wet room, not from a widener |

Rough DRR targets from `meter_stems` `detail: ["depth"]`: **front +10 dB or
more, middle +3 to +8, back 0 or less.** Use them to space the layers apart,
then trust the ordering.

## 1. One shared room

A return bus with the reverb at full wet, fed by post-fader sends:

1. `bus_create`, then `bus_add_effect` with `com.resonance.reverb` and, as
   `preset`, the starting preset §1b gives for the job. The add and the load
   are one undo step, and the preset sets the algorithm with the rest of its
   voicing.

<!-- keys: com.resonance.reverb -->
   Never leave `algorithm` at its default: a new reverb starts on a type
   nobody chose for this job. Start from a preset, or set `algorithm` first,
   before any other key, because each type reads the knobs its own way.
<!-- /keys -->

   Read the plugin back with `bus_plugin_params` to confirm the type and the
   values you are about to change.
2. Set it for a return:

<!-- keys: com.resonance.reverb -->
| Key | On the return | Why |
|---|---|---|
| `mix` | 1.0 (100 % wet) | a return carries only the wet signal; the dry is the track itself. Presets store an insert mix (6-50 %), so set this after loading one |
| `predelay` | from tempo, 20-40 ms (§2), or `predelay_sync` | lets the front layer land before its room |
| `decay` | from tempo (§2), or `decay_sync`, inside the job's range in §1b | a tail still loud on the next downbeat smears the groove |
| `wet_hpf_freq` | the preset's (150-400 Hz) where it has `wet_hpf_on` `On`; otherwise about 600 Hz for a vocal room, 150-250 Hz for a back/wash return | return EQ before the tank: no low-end wash |
| `wet_lpf_freq` | about 10 kHz (6-8 kHz for a darker room), with `wet_lpf_on` `On` | no sizzle; darker reads further away |
| `wet_filter_slope` | `12 dB/oct` or `18 dB/oct` | 12-18 dB/oct is the norm |
| `er_tail_balance` | 0; toward -1 more early reflections (closer), toward +1 more tail (further) | the room's depth crossfade |
| `low_decay_mult`, `high_decay_mult`, `tail_build` | the preset's, then §1b | the shape of the tail |
| `size`, `damping` | leave the preset's unless the user asks for a bigger or smaller space | |
<!-- /keys -->

3. `track_add_send` from each track that needs space, `pre_fader` false (the
   default): moving the track's fader then takes its reverb with it.
   Start from roles.md's first-guess `level_db` per role (kick and bass
   usually get none), then set each send from its layer's DRR target once the
   first depth pass has measured the return (§4).

## 1b. Pick the room by job

One shared room is still the default: choose its type by what leads the song
(the first rows below). Add a second return of another type only when one room
cannot place both the front and the back, or when one part needs a special
effect the shared room should not carry (§4). Every number is a start, checked
by measurement (§7).

<!-- keys: com.resonance.reverb -->
| The return is for | Type (`algorithm`) | Start from | `decay` | `predelay` |
|---|---|---|---|---|
| shared room, a vocal-led song | `Plate` | `Vocal Plate` | 1.2-2.5 s, or `decay_sync` `1/2` to `1 bar` | 20-40 ms, or `predelay_sync` `1/64` or `1/128`, whichever lands there (§2) |
| shared room, a warm or intimate vocal | `Chamber` | `Vocal Chamber` | 1.0-1.8 s | 10-30 ms |
| shared room, a band with the drums leading | `Room` | `Drum Room`, or `Tight Room` for a smaller space | 0.3-0.9 s, or `decay_sync` `1/4` to `1/2` | 0-10 ms |
| shared room for a dry-sounding mix, or a front that must stay dry-sounding | `Ambience` | `Short Ambience` | 0.3-0.8 s (held at 1 s at most) | 0-10 ms |
| the back layer: pads, strings, orchestral parts | `Hall` | `String Hall`, `Warm Hall`; `Cathedral` for a ballad's wash | 2-4 s (a ballad up to 6) | 0-20 ms |
| ambient beds, drones, swelled guitar | `Shimmer` | `Octave Halo`; `Fifth Bloom` for an open fifth; `Shimmer Drone` for an endless halo | 6-20 s | 0-80 ms |
| the same, without the pitched halo | `Hall` | `Ambient Bloom`, with `freeze` to hold a chord | 6-20 s | 0-80 ms |
| snare sheen, its own return | `Plate` | `Snare Plate`, or `Bright Plate` for percussion | 0.8-1.8 s | 0-15 ms |
| a natural snare room, its own return | `Room` | `Snare Tight`; `Snare Ambient` for a bigger bloom | 0.4-0.9 s | 0-15 ms |
| an 80s gated snare, its own return | `Nonlinear` | `80s Gate`, or `Snare Gated` for a tighter one | ignored: set `nl_length` 250-500 ms | 0 ms |
| clean electric guitar, its own return | `Spring`, or `Plate` for a hi-fi version | `Surf Spring` | 1.5-3 s | 0 ms |
<!-- /keys -->

Kick, bass and sub get no reverb on any type: the low end stays dry and mono.

What each type reads, so a move is never made on a knob the type ignores:

<!-- keys: com.resonance.reverb -->
- **Bass and treble decay.** `low_decay_mult` multiplies the decay below
  `low_xover`: above 1 thickens and warms the room (Chamber and Hall presets sit
  at 1.3-1.5), below 1 clears the low end (0.7-0.9 for drum rooms and glue).
  `high_decay_mult` multiplies it above `damping`: lower is darker and reads
  further back (Hall 0.4-0.6), high is a plate's sheen (Plate 0.8-0.9). The
  `algorithm` choices `Plate`, `Room`, `Chamber`, `Hall`, `Ambience` and
  `Shimmer` read all three; `Classic`, `Spring` and `Nonlinear` ignore them, so
  darken those with `damping`.
- **Build.** Only `algorithm` `Hall` and `Shimmer` read `tail_build`, how
  slowly the tail swells in after the reflections: 0 arrives with them, 1
  blooms over about 300 ms. Higher reads further back and softens the attack; the level and the
  decay do not change. 0.4-0.6 for a back layer, 0.8-1 for a bloom.
- **Ambience** holds `decay` at 1 s at most: its space is in the early
  cluster, not in a tail. That is why it can sit on a front part, or at a few
  percent on a whole mix, without reading as reverb.
- **Nonlinear** ignores `decay`, `decay_sync` and `freeze`: `nl_length` is the
  gate (50-1000 ms) and `nl_shape` its envelope (`Gated` flat then cut,
  `Reverse` rising then cut, `Flat` flat then a natural fall). It restarts on
  every transient of its input, so feed it from the snare (and toms) only: a
  sustained part sent to it triggers once and is then gated away.
- **Spring** is mono-in and has no early reflections; it ignores the ER keys,
  the modulation, the decay multipliers, `tail_build` and `freeze`.
  `spring_tension` is the chirp (higher, a longer boing), `spring_drip` the
  extra chirp on each pick attack.
- **Shimmer** pitch-shifts part of its loop: `shimmer_pitch` (`+12` an octave
  halo, `+7` a fifth, `-12` a darker sub-octave) and `shimmer_amount` (how much
  of the loop, 0.3-0.5 in the presets). Keep it on a back return with
  `wet_hpf_freq` 150-250 Hz. With `freeze` on it holds what it has and stops
  climbing.
- **Freeze.** `freeze` holds the tail on every `algorithm` but `Spring` and
  `Nonlinear`, which ignore it.
<!-- /keys -->

Changing the type on a running return crossfades over 50 ms (no click), but
it changes how every other knob is read: re-read the plugin and re-check §7.

## 2. Pre-delay from tempo

A tempo-locked pre-delay sits inside the groove. Use the tempo where the front
part plays (`tempo_events` in `song_summary`), and pick the note value that
lands in the layer's range: 20-40 ms for the front.

| BPM | 1/64 note (60000 / BPM / 16) | 1/128 note (60000 / BPM / 32) |
|---|---|---|
| 60 | 62.5 ms | 31.3 ms |
| 80 | 46.9 ms | 23.4 ms |
| 100 | 37.5 ms | 18.8 ms |
| 120 | 31.3 ms | 15.6 ms |
| 140 | 26.8 ms | 13.4 ms |
| 170 | 22.1 ms | 11.0 ms |

The general form is 60000 / BPM / k with k = 8 (a 1/32 note), 16 or 32. A
1/32 note only fits the front's 20-40 ms at about 188 BPM or faster; below
that it is too long (80 ms at 94 BPM).

<!-- keys: com.resonance.reverb -->
Or let the reverb do the arithmetic: `predelay_sync` `1/64` is the 1/64 column
above and `1/128` the 1/128 column (`1/32`, `1/16` and `1/8` are longer, for a
slap or an effect). While it is not `Off` it overrides `predelay`.
<!-- /keys -->

### Decay from tempo

A room whose tail is still loud on the next strong beat smears the groove.
Size the shared room's decay to the song: decay ≈ n × 60 / BPM seconds, with n
beats. Use n = 1-2 for busy, rhythmic material and n = 4-8 (one to two bars of
4/4) for ballads and sparse songs. The back/wash return of §4 may run longer
than the shared room: that is what it is for.

| BPM | 1 beat | 2 beats | 1 bar (4/4) | 2 bars |
|---|---|---|---|---|
| 60 | 1.0 s | 2.0 s | 4.0 s | 8.0 s |
| 80 | 0.75 s | 1.5 s | 3.0 s | 6.0 s |
| 100 | 0.6 s | 1.2 s | 2.4 s | 4.8 s |
| 120 | 0.5 s | 1.0 s | 2.0 s | 4.0 s |
| 140 | 0.43 s | 0.86 s | 1.7 s | 3.4 s |
| 170 | 0.35 s | 0.71 s | 1.4 s | 2.8 s |

<!-- keys: com.resonance.reverb -->
Set the result as `decay` on the return, inside the job's range in §1b. On
every type but `algorithm` `Classic` it is the mid-band T60 and measures within
a few percent of the knob (`Shimmer` within about 15 %). Keep the treble
shorter as `decay` grows (`high_decay_mult` down, or `damping` lower on the
types that ignore it): a long, bright tail reads as harsh rather than deep.

Or sync it: `decay_sync` sets the T60 to a length at the song's tempo, `1/4`
one beat, `1/2` two beats, `1 bar`, `2 bars` or `4 bars` (a bar in the song's
meter); any choice but `Off` overrides the decay knob. The synced values follow
the tempo the song is playing at, so they track a tempo map by themselves;
with no tempo from the host they fall back to the knobs. They are still
subject to each type's limits: `algorithm` `Ambience` holds 1 s at most, and
`Nonlinear` ignores `decay_sync` (its length is `nl_length`). Confirm a synced
decay by measuring it (§7).
<!-- /keys -->

## 3. Duck the room from the lead

The lead stays clear while it sings and blooms in the gaps. On the room return:

1. `bus_set_sidechain {bus_id: <room>, plugin_id: "com.resonance.reverb",
   source_track_id: <lead>}`. The key only drives the ducker; it never reaches
   the output. With no key the ducker listens to the reverb's own input (the
   sum of the sends), which ducks the room from everything.
2. Set the ducker:

<!-- keys: com.resonance.reverb -->
| Key | Start | Notes |
|---|---|---|
| `duck_amount` | 0.15-0.25 | 0 is off; 1 is 24 dB of reduction, so 0.2 is about 5 dB |
| `duck_threshold` | -30, then adjust | the key's level where ducking starts: under the lead's sung level, over its gaps |
| `duck_attack` | 10-30 ms | |
| `duck_release` | 100-300 ms | the room swells back in over this after each phrase |
<!-- /keys -->

Verify with a raw compare on the return, over a sung passage and over a gap:
`meter_snapshot {target: {bus_id}, range}` before ducking, then
`meter_compare {a: snapshot_id, match: "none"}` after. During the phrase the
return's `lufs_integrated` should fall 3-6 dB; over a gap, barely at all. The
matched compare would hide exactly the level change you are checking, which is
why this one uses `match: "none"`.

## 4. Per-layer sends and a second return

Set each track's send (`track_set_send`) from its layer's DRR target. The
estimate is arithmetic on the send, DRR = −(send + `return_gain_db`) for a
single post-fader send, so read the return's `return_gain_db` from the track's
`sends` in the depth pass and set the send to −(target) − `return_gain_db`
(roles.md works an example). Raising a send by 3 dB lowers that track's
`drr_db_estimate` by 3 dB. Then check the ordering holds.

If one room cannot place both the front and the back, add a second return
rather than pushing sends to extremes. The same goes for a part that needs a
room of its own type (§1b):

<!-- keys: com.resonance.reverb -->
| Return | Type (`algorithm`) | Fed by | Settings |
|---|---|---|---|
| Room (shared) | by what leads the song (§1b) | everything that needs space | as above |
| Back / wash | `Hall`, or `Shimmer` for ambient beds | pads, strings, FX, textures | longer `decay`, `tail_build` 0.5-1, `er_tail_balance` +0.3 to +0.6, `wet_lpf_freq` 6-8 kHz, `wet_hpf_freq` 150-250 Hz, `predelay` 0-10 ms |
| Front | `Ambience` | a front part that needs air but must not sound wet | `decay` 0.3-0.8 s, `predelay` 0-10 ms |
| Snare | `Plate`, `Room` or `Nonlinear` (§1b) | the snare (and toms) only | the preset's; `wet_hpf_freq` 200-350 Hz keeps the kick out |
| Guitar | `Spring` | the clean electric guitar only | the preset's |
<!-- /keys -->

The back parts then send to both, or only to the wash. Keep the front on the
shared room only. Each extra return is one more space: stop at two or three.

Delay throws on phrase ends keep a lead upfront better than more reverb: a
synced delay return (`com.resonance.delay`), fed by a send from the lead.

<!-- keys: com.resonance.delay -->
Start from `Dotted Eighth` or `Quarter Note`, set `mix` to 1.0 on the return,
`lo_cut` and `hi_cut` to keep the repeats out of the low end and the air,
`feedback` low (0.2-0.35), and `duck_amount` so the repeats sit under the lead
and bloom in its gaps.
<!-- /keys -->

## 5. Darken the back

Back parts get less top than front parts: a high shelf of -1 to -3 dB above
6-8 kHz on the part (or the colour plugin's output tilt, when it is on the part
for character anyway). Front parts keep their top. Check the ordering of
`hf_tilt_db` (energy 6-16 kHz over 1-4 kHz): it should fall front → back.

## 6. Verify the ordering

`meter_stems` with `detail: ["depth"]`:

- `drr_db_estimate`: front > middle > back, and roughly in the target bands
  above. `dry_only` tracks (no sends) rank as driest.
- `layer_hint` is the tertile of this one pass. It should agree with your
  layer table; where it does not, the send (or the role) is wrong.
- `hf_tilt_db` falling front → back.

Caveats: only a track's own sends count (a track feeding a bus that sends to
the room reads as dry); automated send rides are not in the estimate; it costs
one extra render per return and per sending track.

## 7. Verify the tail

Decay from tempo is arithmetic on a knob; the decay detail measures what the
room actually does. Do it once per return, after its settings are final.

1. **Pick a sender and a stop.** Measure a **track that sends** to the
   return, never the return bus: a track's stem carries its sends' returns
   back, while a `{bus_id}` target hears only tracks routed into the bus and
   reads a send-fed return as silent. Choose a range that ends in silence
   after the track stops: from a held note through the gap before its next
   entry, or its last note with the default range (an explicit range is
   clamped to the song's end and cuts the tail). Sustained material (a pad, a
   held vocal note) reads cleanest; a pluck adds its own decay.
2. **Measure.** `meter_measure {target: {track_id}, range, detail: ["decay"]}`,
   or `meter_stems` with `detail: ["decay"]` for every sender at once.
3. **Read** (fields in reading-meters.md, "Decay"):
   - `found` and `clean` true, and `stop` the stop you meant. Otherwise `note`
     says why: extend the range (`ends` is `range_end`), or pick a stop that
     other notes do not cover.
   - `t30_seconds` within about 10 % of the decay you intended: the knob, or
     for a synced decay its beats × 60 / BPM. Further off, read the plugin
     back with `bus_plugin_params`: the type, a sync you forgot, or Ambience's
     1 s ceiling. On a gated Nonlinear return T30 means little; read
     `tail_20db_seconds` against the gate length instead.
   - `stop_seconds` plus `tail_20db_seconds` falls before the next downbeat
     (song seconds from the tempo) on rhythmic material: the tail has cleared
     20 dB before the groove needs the space. Later than that, shorten the
     decay a step (a sync one choice down) or duck the return (§3). A
     ballad's room and the back/wash return may ring past it by design.
   - `bands`: the 125 Hz and 250 Hz `t30_seconds` against the 1 kHz one
     follow the bass multiplier you set. Bass much longer than intended is a
     muddy room: lower the bass decay or raise the return's high-pass.
   - `edt_seconds` shorter than T30 is normal on a track (its dry signal
     stops at once).

Tails longer than about 15 s (a frozen or drone return) cannot be measured
this way: check those by the depth ordering only.

## Stop rules

- The ordering holds and the targets are roughly met: stop. Wetter is not
  deeper once the ordering holds.
- A front part's DRR under about +6: it will read as distant; take its send down.
- The room's return is louder than its loudest source during the song: too wet
  overall; take the return fader down before touching individual sends.
