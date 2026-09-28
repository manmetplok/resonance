# Depth: layers, the room, the numbers

**Depth is contrast between layers**, not an absolute setting. Verify the
*ordering* (front drier than middle, middle drier than back), never a single
track's number against a target.

## The cues, strongest first

| Cue | Front | Back | Lever here |
|---|---|---|---|
| Level | louder | quieter | the fader (the `mixing` skill's balance) |
| Direct-to-reverberant ratio (DRR) | dry | wet | the send level into the shared room |
| Pre-delay (pop/rock convention) | long, 20-40 ms: the voice lands before its room | short, 0-10 ms | the room's `predelay`, or a second return |
| Early reflections vs tail | ER carry position | tail carries the room's size | the room's ER/tail balance |
| High-frequency content | bright | darker | a high shelf or the colour plugin's output tilt on back parts; the room's wet low-pass |
| Transients | sharp | softer | less compression on front parts; softer attack on back parts |
| Width of the *source* | as recorded | narrower | a distant source reads narrower. A back-layer *bed* (pads) can still be wide: its width comes from the wet room, not from a widener |

Rough DRR targets from `meter_stems` `detail: ["depth"]`: **front +10 dB or
more, middle +3 to +8, back 0 or less.** Use them to space the layers apart,
then trust the ordering.

## 1. One shared room

A return bus with the reverb at full wet, fed by post-fader sends:

1. `bus_create`, then `bus_add_effect` with `com.resonance.reverb`.
2. Start from a preset with `bus_load_plugin_preset`, then set it for a return:

<!-- keys: com.resonance.reverb -->
| Start from | Suits |
|---|---|
| `Tight Room` | dense, close material; a short shared room |
| `Vocal Plate` | a vocal-led song |
| `Warm Hall` | a slower, sparser song |

| Key | On the return | Why |
|---|---|---|
| `mix` | 1.0 (100 % wet) | a return carries only the wet signal; the dry is the track itself |
| `predelay` | from tempo, 20-40 ms (table below) | lets the front layer land before its room |
| `wet_hpf_on`, `wet_hpf_freq` | `On`, about 600 Hz | return EQ before the tank: no low-end wash |
| `wet_lpf_on`, `wet_lpf_freq` | `On`, about 10 kHz (6-8 kHz for a darker room) | no sizzle; darker reads further away |
| `wet_filter_slope` | `12 dB/oct` or `18 dB/oct` | 12-18 dB/oct is the norm |
| `er_tail_balance` | 0; toward -1 more early reflections (closer), toward +1 more tail (further) | the room's depth crossfade |
| `decay`, `size`, `damping` | leave the preset's unless the user asks for a bigger or smaller space | |
<!-- /keys -->

3. `track_add_send` from each track that needs space, `pre_fader` false (the
   default): moving the track's fader then takes its reverb with it.
   Starting `level_db` per role: roles.md. Kick and bass usually get none.

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
1/32 note is too long for most front vocals above about 90 BPM.

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

Adjust each track's send (`track_set_send`) until the DRR ordering holds. The
estimate is arithmetic on the send: raising a send by 3 dB lowers that track's
`drr_db_estimate` by about 3 dB.

If one room cannot place both the front and the back, add a second return
rather than pushing sends to extremes:

| Return | Fed by | Settings |
|---|---|---|
| Room (shared) | everything that needs space | as above |
| Back / wash | pads, FX, textures | longer `decay`, `er_tail_balance` +0.3 to +0.6, `wet_lpf_freq` 6-8 kHz, `predelay` 0-10 ms |

The back parts then send to both, or only to the wash. Keep the front on the
shared room only.

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

## Stop rules

- The ordering holds and the targets are roughly met: stop. Wetter is not
  deeper once the ordering holds.
- A front part's DRR under about +6: it will read as distant; take its send down.
- The room's return is louder than its loudest source during the song: too wet
  overall; take the return fader down before touching individual sends.
