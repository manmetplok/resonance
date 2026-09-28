# Character: warmth, harshness, tilt

The mixing skill's character pass, in full. Field meanings are in
`${CLAUDE_SKILL_DIR}/references/reading-meters.md`; this file is the judgement.

Two rules come before everything else:

- **Loudness confounds every judgement.** A saturator raises RMS, and louder
  reads as warmer and better on every number. Judge each move with
  `meter_compare` against a `meter_snapshot`, which gain-matches B to A, never
  by re-measuring and comparing raw numbers.
- **Harshness before warmth.** Fix a presence peak first. Saturating a harsh
  mix makes it louder and harsh.

## Vocabulary: the user's word → the signal → the lever

| Word | What it means in the signal | Levers |
|---|---|---|
| **Warm** | (a) low-order, even-dominant harmonics (H2 ≥ H3, the series falling ≥ 6 dB per order) at low level; (b) a slightly steeper tilt: a little more 100-400 Hz, a little less 2-5 kHz and above 10 kHz; (c) softer transients, crest down 0.5-2 dB | even-dominant saturation on busses, tape/transformer voicing, tilt or shelves, de-harsh, glue |
| **Harsh / cold / digital** | energy peaks in 2-5 kHz; odd, slowly decaying harmonics; aliasing | a static or dynamic cut in 2-5 kHz, a softer or even-dominant curve, more oversampling |
| **Muddy** | a 200-500 Hz build-up, often from many wide sources | cut the low mids, on the sides first on wide sources |
| **Dull** | warmth pushed too far: tilt past the target, centroid down more than 15% | back the last move off; an air shelf |
| **Thin** | too little 100-400 Hz relative to presence | an LF lift+dip, or a broad low-mid bell up |
| **Glued / cohesive** | parts sharing one envelope: light, slow bus compression plus light bus saturation | bus compressor at 2:1, colour on the busses |

Width words (wide, narrow, mono-safe) and depth words (deep, upfront, distant)
belong to the `spatial` skill.

## The numbers (all from `detail: ["spectrum", "dynamics"]` or `meter_probe`)

| Proxy | Field | "Warmer" means | Stop when |
|---|---|---|---|
| Spectral tilt | `tilt_db_per_oct` | more negative by 0.5-1 (e.g. -4.0 → -4.8). Pop averages -4.5 to -5 | it passes about -5.5, or moves more than 1.0 from baseline |
| Centroid | `centroid_pct` (compare delta) | down 5-15% | down more than 15%: that is dull |
| Low-mid / presence | `lowmid_presence_db` | up 1-2 dB | up more than 3, or up with the tilt already steep: mud |
| Presence peakiness | `presence_peakiness_db` | down | — |
| Air | `air_ratio_db` | slightly down | down more than about 2 dB |
| Crest | `crest_db` (compare delta) | down 0.5-2 dB | down more than 2 dB |
| Peak-to-short-term | `psr_db` (absolute) | — | under 8 dB |
| Harmonic signature | `thd_pct`, `h2_h3_db`, `decay_db_per_order`, `aliasing_floor_dbc` from `meter_probe` | H2 over H3 (`h2_h3_db` > 0), decay ≥ 6 dB per order, aliasing ≤ -90 dBc | THD over the placement's band (below) |

THD targets for a character stage, by placement, read from a `meter_probe`
at `level_dbfs: -18` (the level the presets are voiced at; see step 3):

| Placement | `thd_pct` | Roughly |
|---|---|---|
| Master | 0.1-1 % | -60 to -40 dBc |
| Bus | 0.5-3 % | |
| Single track | 3-10 % | |

## The warmth procedure

Measure → decide → act → verify, one class of move per step. Stop at the first
step that gets the mix where it needs to be.

### 1. Baseline

`meter_snapshot` on the master (it keeps all three details by default). Note
`tilt_db_per_oct`, `centroid_hz`, `lowmid_presence_db`,
`presence_peakiness_db`, `peaks`, `crest_db`, `psr_db`, the stereo bands'
correlation, and `lufs_integrated`. Then `meter_stems` with
`include_busses: true` and `detail: ["spectrum"]` to find *where* a problem
lives. Busses overlap their members: never add a bus to its tracks.

### 2. De-harsh

Decide: a `peaks` entry between 2 and 5 kHz with `excess_db` of 3 or more, or
`presence_peakiness_db` clearly above the other entries of the same pass, is a
harshness problem. Find the stem or bus whose own `peaks` show the same
frequency.

Act: an EQ bell on that stem or bus.

<!-- keys: com.resonance.eq -->
| Move | Keys and values |
|---|---|
| Enable a band | `band{n}_enabled` `On`, `band{n}_kind` `Bell` |
| Static cut | `band{n}_freq` at the peak, `band{n}_gain` -1 to -3, `band{n}_q` 2-4 |
| Cut only when it is hot | `band{n}_kind` `Bell`, `band{n}_gain` 0, `band{n}_dyn_on` `On`, `band{n}_dyn_threshold` a few dB under the region's loud level, `band{n}_dyn_ratio` 2-4, `band{n}_dyn_attack` 5-15 ms, `band{n}_dyn_release` 100-200 ms |
| Cut one part when another plays (unmasking) | the dynamic bell above, plus `band{n}_dyn_sc` `On` on the EQ of the part being cut |
| Judge at matched level | `auto_gain` `On` |
<!-- /keys -->

A bell at 0 dB with dynamics on is a pure de-harsh cut: it does nothing until
its region is loud. Prefer it for a resonance that comes and goes (a vocal's
sibilant phrases, cymbal crashes). Keep dynamics to bells and shelves; the
tilt and lift+dip kinds in step 4 are static moves. On the master, the
mastering chain has its own resonance suppressor (the `mastering` skill's
de-harsh stage); on a stem or a bus, this bell is the tool.

For the keyed band, route the masking part into the EQ with `track_set_sidechain`,
naming the plugin `com.resonance.eq` explicitly: an unqualified call prefers a
compressor or gate and never picks the EQ over them.

Verify: `meter_compare {a: snapshot_id}` — `presence_peakiness_db` down, that
peak's third-octave band down, and `tilt_db_per_oct` barely moved.

### 3. Warmth on the busses, not the master

Decide: after de-harsh, the tilt is still flatter than the target, or the user
asked for warmth. Work bus by bus: drums, bass, music, vocals — whichever exist
(`song_tracks`; create busses with `bus_create` + `track_set_output` if the song
has none and the user agrees).

Act: `bus_add_effect` with `com.resonance.color`, then start from a preset with
`bus_load_plugin_preset`:

<!-- keys: com.resonance.color -->
| Bus | Start from | Voicing it uses |
|---|---|---|
| Mix or music bus | `Bus — Warm Glue` | bus THD band |
| Drum bus | `Drums — Tape 15` | `mode` `Tape` at `speed` `15 ips`, bus THD band |
| Bass track or bus | `Bass — Iron` | `mode` `Transformer`, track THD band |
| Lead vocal track | `Vocal — Tube Air` | `mode` `Tube`, track THD band |
| Master (step 6 only) | `Master — Subtle Tape` | master THD band |

The controls, when a preset needs moving:

| Key | What it does | Warmth setting |
|---|---|---|
| `mode` | voicing: `Tube` (biased, H2-dominant), `Tape` (soft curve + head bump + level-dependent HF loss), `Transformer` (drives the lows harder, sub-sonic HPF), `Console` (odd, very clean, for glue at low drive), `Warm` (one-polarity: even harmonics only) | `Tube`, `Tape` or `Warm` for warmth; not `Console` |
| `drive` | how hard the curve is hit, 0-1 | set by probe, below |
| `bias` | asymmetry, 0-1: more means more H2 | 0.5-0.7 for even-dominant; ignored in `mode` `Console` |
| `response` | tilt of the *drive*, dB: positive saturates the lows more | +2 to +6 for low-end weight without fizz |
| `tone` | output tilt, ±6 dB, positive brighter | -0.5 to -1.5 for a darker result |
| `mix` | dry/wet | 0.2-0.5 on a bus |
| `auto_gain` | matches the output's loudness to the input | leave `On`: it is what makes the move judgeable |
| `oversample` | `Off` / `2x` / `4x`, latency-free | `2x`; `4x` if the aliasing probe fails |
| `speed` | `7.5 ips` / `15 ips` / `30 ips`, only in `mode` `Tape`; moves the head bump up with speed | `speed` `15 ips` |
| `flutter` | only in `mode` `Tape`; 0 bypasses it | 0 unless asked for wobble |
| `tape_quality` | `Standard` or `HQ` (hysteresis; costs CPU) | `Standard` |
<!-- /keys -->

Color is **not** inert when inserted: its defaults already drive the signal.
Load a preset or set every control before measuring.

Set drive to a THD target with `meter_probe`, not by knob position. The THD
bands, and the presets that land in them, are defined for a **-18 dBFS sine**,
so probe at that level; pass it explicitly, because the tool's own default is
-12:

1. `meter_probe {target: {bus_id}, level_dbfs: -18}`.
2. `thd_pct` under the bus band (0.5-3 %)? raise the drive by about 0.05 and
   probe again; over it, lower. Three or four probes is normal.
3. Check the signature: `h2_h3_db` above 0 (even-dominant), `decay_db_per_order`
   6 or more. Odd-dominant means the wrong mode or too little bias.
4. `meter_probe {level_dbfs: -18, freq_hz: 5000}` once: `aliasing_floor_dbc`
   must be -90 or lower. If not, raise the oversampling or lower the drive.
5. Optionally, one probe at the level the bus really peaks at (its
   `true_peak_db` from the `meter_stems` pass with `include_busses: true`,
   within `level_dbfs`'s -80..0) shows how hard the loudest moments hit. It
   reads higher than the -18 figure; report it, but set the drive by the -18
   one.

<!-- keys: com.resonance.color -->
The knobs those steps turn are `drive`, `mode`, `bias` and `oversample`.
<!-- /keys -->

Verify: `meter_compare {a: snapshot_id}` on the master. Keep the move only if
`tilt_db_per_oct` went more negative or `lowmid_presence_db` rose, and none of
the stop rules below fired.

### 4. Tone

Decide: the tilt still needs to move, or the top is bright or the bottom thin.

Act: one EQ band per move on the bus (or the master's tonal stage in the
`mastering` skill).

<!-- keys: com.resonance.eq -->
| Move | Keys and values |
|---|---|
| Gentle darker tilt | `band{n}_kind` `Tilt`, `band{n}_freq` (the pivot) about 1000, `band{n}_gain` -0.5 to -1 (the top goes down by that, the bottom up by it) |
| Softer top | `band{n}_kind` `High Shelf` at 10-12 kHz, -0.5 to -1.5 dB |
| Low weight without mud | `band{n}_kind` `LF Lift+Dip` at 60-100 Hz, +1 to +2 dB: it lifts there and dips at 3.5× that (210-350 Hz) by half as much |
| Broad low-mid body | `band{n}_kind` `Bell` at 150-300 Hz, +0.5 to +1 dB, `band{n}_q` 0.5-0.8, only if the low mids are not already muddy |
| Air instead of brightness | `band{n}_kind` `Air` at 10-20 kHz, +1 to +2 dB |
| Mud on a wide source | `band{n}_kind` `Bell` at 250-500 Hz, -1 to -3 dB, `band{n}_ms` `Side` |
<!-- /keys -->

Verify: `meter_compare` again; the move should shift `tilt_db_per_oct` by the
amount intended and leave `presence_peakiness_db` alone.

### 5. Glue

Decide: the parts still sit separately (high `crest_db`, busses whose loudness
wanders independently), or the user asked for glue.

Act: `com.resonance.compressor` on the mix or music bus.

<!-- keys: com.resonance.compressor -->
Start from the preset
`Bus — Auto Glue`, or by hand: `ratio` 2, `attack` 10-30 ms,
`release_mode` `Auto`, `threshold` set so the bus crest falls by no more than
about 2 dB, `auto_makeup` `On`. Parallel with `mix` 0.5-0.7 if it pumps.
<!-- /keys -->

Verify: `meter_compare` — `crest_db` down 0.5-2 dB, `psr_db` still 8 or more.

### 6. Master character — only if still sterile

"Sterile" has a number: after steps 2-5, the master's `tilt_db_per_oct` is
still flatter than about -4.5 **and** nothing in the path adds harmonics: a
`meter_probe {level_dbfs: -18}` of the master chain **and** of every mix bus
(`{target: {bus_id}}`, each bus from `song_tracks`) shows `thd_pct` under
0.1 %. A bus that already carries colour means the mix is not sterile, whatever
the master probe says: the busses are where step 3 put the warmth. Only then is
it the `mastering` skill's saturator stage, at
the master THD band (0.1-1 %), before the clipper and limiter, never after the
limiter. Parallel at 10-30 % if in doubt.

### 7. Accept or roll back

Final `meter_compare {a: snapshot_id}` against the baseline from step 1.
Accept the pass only if the tilt moved the warm way **and**:

- `psr_db` is still 8 or more;
- no probe showed new aliasing (`aliasing_floor_dbc` ≤ -90);
- the stereo deltas show low-band `correlation` and `mono_loss_db` no worse;
- true peak is still well under 0 dBFS (mix stage) or ≤ -1 dBTP (master).

Otherwise `edit_undo` back to the last move that passed.

## Stop rules

Stop adding character, and back off the last move, when any of these fires:

- `crest_db` falls faster than the loudness it bought (the compare's
  `match_gain_db` shrinks while the crest delta grows), or by more than 2 dB
  in total.
- `tilt_db_per_oct` is past the target (about -5.5), or `centroid_pct` is down
  more than 15%: that is dull, not warm.
- `psr_db` under 8.
- `thd_pct` over the placement's band, or `h2_h3_db` gone negative.
- The user's word changed from "warm" to "muddy", "dark" or "boxy".

## Report

One line per move: *stage → the number that justified it → what it changed
(compare delta) → what it cost*. For example: "music bus, Color preset Bus — Warm
Glue at drive 0.30 → tilt -4.1 → -4.6 dB/oct, THD 1.4 %, crest -0.8 dB". Every
move is an ordinary undo step.
