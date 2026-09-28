# Reading a resonance measurement

Field semantics for `mcp__resonance__meter_measure` and
`mcp__resonance__meter_stems`. Read this before your first diagnosis — several
of these are counter-intuitive, and one of them (`bands`) is routinely misread
as an absolute.

## The unit

**1 LU == 1 dB.** A difference in LUFS is a difference in dB, so a track sitting
6 LU under another needs `mixer_set_volume_db` moved by +6 to meet it. This is
the single most useful fact here.

## Fields

| Field | What it means | How to read it |
|---|---|---|
| `lufs_integrated` | Gated loudness | **Gated**: a part playing in 2 of 9 sections reports how loud it is *while it plays*, not an average diluted by its silence. That gating is what makes it usable for balance. |
| `lra` | Loudness range | How much the loudness varies across the measured span. Very low on a full mix means it is already heavily compressed. |
| `true_peak_db` | Inter-sample peak, dBTP | -1 dBTP is Spotify's recommendation, -2 is safer through lossy codecs, which push peaks up. |
| `sample_peak_db` | Plain sample peak | Always ≤ true peak. Only present with `source: "render"`. |
| `crest_db` | Peak-to-average | Under 8 dB is squashed. Over 20 dB is essentially uncompressed. Between is normal and genre-dependent. |
| `correlation` | Stereo phase | Near +1 mono-identical, near 0 wide, **negative means anti-phase content a mono listener loses**. Reads 0 for hard-panned mono too: ask for `detail: ["stereo"]` and check `one_sided`. |
| `mono_penalty_db` | Level lost on mono fold-down | Read alongside `correlation`; this is the actual cost. |
| `clipped_samples` | Count | Above 0 on the master is audible clipping. Not a matter of taste. |
| `bands` | `low` / `mid` / `high` / `air` | **Raw energy shares summing to 1.0.** |
| `measured_seconds` | Span actually measured | Identical across every entry of one `meter_stems` pass, which is what makes them comparable. |
| `soloed_track_ids` | Present on master results | Non-empty ⇒ the master numbers describe only those tracks, not the mix. Per-track entries still read correctly. |

## `bands` is relative, always

The four shares sum to 1.0 by construction. There is no absolute rule about what
a healthy `low` share is — it depends entirely on the arrangement. Use them
**only** as:

- track vs. track ("both of these put 60% of their energy in `low`, they are
  fighting"), or
- master vs. a reference measurement of a track you trust.

Comparing a band share against a remembered number is how you talk yourself into
an EQ move the song did not need.

## Detail blocks (opt-in)

Both tools take `detail`, a list of any of `"spectrum"`, `"stereo"` and
`"dynamics"`, e.g. `detail: ["spectrum", "stereo"]`. Each named detail adds one object to
every result (the master and every `meter_stems` entry), named after the detail.
Without `detail` the reply is exactly the standard fields above. Details need a
render: asking for one with `source: "live"` is refused. They cost one spectral
analysis per entry, not an extra render.

Judge any change in a detail number at **matched loudness**. A louder mix reads
brighter and "better" in every one of these.

### `spectrum`: tone and warmth

Read off the stereo long-term average spectrum (left and right power averaged).
Unlike `bands`, it keeps anti-phase side content.

| Field | What it means | How to read it |
|---|---|---|
| `third_octave` | 31 ISO 1/3-octave band levels, 20 Hz to 20 kHz, dB (a full-scale sine in the band reads 0) | Centres: 20, 25, 31.5, 40, 50, 63, 80, 100, 125, 160, 200, 250, 315, 400, 500, 630, 800, 1k, 1.25k, 1.6k, 2k, 2.5k, 3.15k, 4k, 5k, 6.3k, 8k, 10k, 12.5k, 16k, 20k. **Pink noise reads flat**, so a band sticking up is more energy than pink. -120 means no energy. |
| `tilt_db_per_oct` | Slope of the power density over 100 Hz to 10 kHz | Pink noise -3.0, white 0. Commercial pop averages about **-4.5 to -5**. More negative is darker/warmer. "Warmer" is a move of roughly -0.5 to -1 (e.g. -4.0 to -4.8). It equals the slope of `third_octave` minus 3.01. |
| `centroid_hz` | Power-weighted mean frequency | Falls about 5-15% when a mix gets warmer at matched loudness. Falling further reads as dull. |
| `lowmid_presence_db` | Energy 150-500 Hz over 2-5 kHz, dB | Rises 1-2 dB for "warmer". A big rise with a steep tilt is mud, not warmth. |
| `presence_peakiness_db` | Crest of the 1/6-octave bands inside 2-5 kHz (loudest over mean), dB | 0 is perfectly even. High means one presence resonance, the usual cause of harshness. Fix it before adding warmth. |
| `air_ratio_db` | Energy 8-16 kHz over the total, dB | Always negative. Drops slightly for "warmer". |
| `peaks` | Up to 5 narrow resonances, `{freq_hz, excess_db}`, strongest first | `excess_db` is how far a 1/6-octave band stands above the octave either side of it. A peak must clear 1 dB and the measurement's own noise, so a short range reports fewer and only larger peaks. Empty is normal. |

### `stereo`: width and mono safety

| Field | What it means | How to read it |
|---|---|---|
| `bands` | 8 bands, edges 20, 60, 150, 400, 1k, 2.5k, 5k, 10k, 20k Hz, each `{lo_hz, hi_hz, correlation, side_mid_db, mono_loss_db}` | All three figures are `null` for a band with (almost) no content. |
| band `correlation` | L/R correlation inside the band | Healthy: **≥ +0.9 below 150 Hz**, ≥ +0.5 from 150 Hz to 1 kHz, ≥ 0 above 1 kHz. `null` when the band is one-sided. |
| `side_mid_db` | Side over mid power in the band, dB | The linear "width" number. -60 is mono, 0 is hard-panned or fully uncorrelated, +60 is anti-phase. Healthy: below 150 Hz ≤ -20; mids -12 to -4; highs -8 to -2; a whole mix sits around -10 to -4. |
| `mono_loss_db` | Level the band loses folded to mono, dB (negative is a loss) | 0 for mono, about -3 for uncorrelated or hard-panned content, -60 for anti-phase. A mix should lose ≤ 3 dB, a track ≤ 6. |
| `correlation_windows` | 400 ms windows: `{windows, pct_below_0_3, worst, worst_at_seconds}` | `windows` counts non-silent, two-sided windows. Warn when `pct_below_0_3` is over 10. `worst` below -0.1 is a phase fault; `worst_at_seconds` (from the start of the range) says where to look. `null` when no window could be counted. |
| `balance_db` | Left over right energy, dB | Positive leans left. A mix should sit within ±1 dB. ±60 means one channel is silent. |
| `one_sided` | One channel is at least 40 dB below the other | Hard-panned mono. See below: correlation is undefined then. |
| `haas_lag_ms` | A static delay between the channels of 1-35 ms | Positive means the right channel is late. It combs in mono, with nulls at (2k+1)·1000/(2·lag) Hz: 10 ms puts the first null at 50 Hz. `null` is the normal, healthy answer. |

**Side/mid and correlation are one number in two spellings.** With equal L/R
energy, `r = (1 − ρ)/(1 + ρ)`, where `ρ = 10^(side_mid_db/10)` is the side/mid
power ratio: side_mid -10 dB is r ≈ 0.82, -6 dB is r ≈ 0.60, 0 dB is r = 0.
Reason about width in `side_mid_db`, which moves linearly as width does. Treat
`correlation` as the fault detector.

**Hard-panned mono is one-sided, not 0 and not +1.** With one channel silent,
correlation is 0/0; meters disagree on what to print, and both guesses mislead.
Here `one_sided` is true, every band `correlation` is `null`, and `balance_db`
(+60 or -60) says which side carries the signal. The top-level `correlation`
field still reads 0 in that case. **Do not read it as "wide"** when `one_sided`
is true.

### `dynamics`

| Field | What it means | How to read it |
|---|---|---|
| `plr_db` | `true_peak_db` minus `lufs_integrated` | 8-12 dB is typical for a finished master. Lower means limited hard. |
| `psr_db` | `true_peak_db` minus `lufs_short_max` (the loudest 3 s) | Keep it at **8 or more**. Stop adding density or limiting when it drops under that. `null` for a range shorter than 3 s. |

## `null` never means zero

A `null` field means the number does not exist for that measurement:

- the range was silent, or too short for that meter's window, or
- `source` was `"live"`.

Check `source` before trusting any field.

## `render` vs `live`

| | `source: "render"` (default) | `source: "live"` |
|---|---|---|
| What it is | Offline render of the range | The master meter as it plays |
| Targets | master, `{track_id}`, `{bus_id}` | master only |
| Needs | stopped transport, no bounce in flight | playback |
| Deterministic | yes | no |
| Supplies | everything | `lufs_integrated`, `lra`, `true_peak_db` only |

The three figures `live` does supply are **session-cumulative** — they cover
everything played since the audio engine started, not the range you asked for.
So: `live` answers "how is this session going", `render` answers anything about
a passage. `crest_db` and phase can only be judged from `render`.

## Target loudness by stage

| Stage | Integrated | True peak |
|---|---|---|
| Mix, handed to mastering | **-23..-18 LUFS** | well under 0 dBFS |
| Finished, limited master | ~-14 LUFS ceiling of usefulness | ≤ -1 dBTP, -2 safer |

Streaming platforms normalise, so mastering past about -14 LUFS buys nothing: it
gets turned back down on playback and you keep only the squashed dynamics.

## A starting balance convention

Validated in the field, offered as a *tripwire, not a rule*. Its value is
catching a track that drifted far from where you meant it, not dictating the
arrangement:

| Role | Relative to drums |
|---|---|
| Drums | 0 LU (reference) |
| Bass | -2 |
| Lead | -4 |
| Rhythm guitar | -6 |
| Texture | -11 |
| Pad | -13 |
| FX | -18 |

If a track is 2 LU off this, that is the arrangement. If it is 12 LU off, ask
whether that was deliberate.

## Before and after: `meter_snapshot` and `meter_compare`

Loudness confounds every judgement: a louder mix reads warmer, brighter and
"better" on every number above. Judge a move by comparing at matched loudness:

1. `meter_snapshot` before the move. It returns a `snapshot_id` plus the stored
   measurement, which is your baseline. By default it keeps all three detail
   blocks.
2. Make the move.
3. `meter_compare {a: snapshot_id}`. `b` defaults to `"current"`, which
   re-renders exactly the snapshot's target and sample range.

The result's `deltas` are **B minus A with B gain-matched to A's integrated
loudness** (`match: "lufs"`, the default). `match_gain_db` is the gain that was
applied to B: a move that made things 1.5 dB louder reads about -1.5 here. That
is the loudness you would have to give back to hear the move fairly.

| Delta group | Moves with the match gain? | Read it as |
|---|---|---|
| `lufs_integrated`, `lufs_short_max`, `lufs_momentary_max`, `true_peak_db`, `sample_peak_db`, `third_octave` | Yes | `lufs_integrated` is ~0 by construction. `true_peak_db` up at matched loudness means peakier. A `third_octave` band up means that band grew relative to the rest. |
| `crest_db`, `lra`, `correlation`, `mono_penalty_db`, `bands`, `tilt_db_per_oct`, `centroid_hz`, `centroid_pct`, `lowmid_presence_db`, `presence_peakiness_db`, `air_ratio_db`, `plr_db`, `psr_db`, every stereo delta | No: a pure gain cannot change them | Changes in character. |
| `clipped_samples` | Reported as measured | A clip count cannot be re-derived at another gain. |

Matching is arithmetic on the stored numbers, which is exact for a gain, so an
unchanged state compares to all zeros and a pure +3 dB fader move compares to
≈0 with `match_gain_db` ≈ -3.

A warmer move looks like this: `tilt_db_per_oct` more negative (about -0.5 to
-1), `lowmid_presence_db` up 1-2, `presence_peakiness_db` down, and
`centroid_pct` down 5-15. Its cost shows as `crest_db` falling (stop past -2).
Also check the absolute `psr_db` stays ≥ 8. For width, check that the stereo
`side_mid_db` deltas rose above 150 Hz while the low bands' `correlation` and
`mono_loss_db` did not get worse.

Snapshots live in the app's memory for the session. They are not saved with the
project, a restart loses them, and past 32 the least recently used is evicted,
which reads as "not found". Two snapshots compare instantly with no render. The
solo state is part of what was measured, so compare like with like.

## Harmonic signature: `meter_probe`

`meter_probe {target, freq_hz, level_dbfs, imd}` runs a sine through an insert
chain (the master's, a bus's, or a track's inserts; a track's instrument is not
part of it) and reports what the chain adds. Use it to set a saturator's drive
to a THD target instead of reading a knob.

It is safe to run at any time. It builds a **fresh clone** of each plugin from
the live plugin's current saved state and probes the clones. The live plugins
are only read, so playback, automation, undo and every plugin's running state
are untouched. The clone gets no automation (it probes the current values), and
a sidechain key hears silence. `stages` lists what was probed. A stage with
`state_copied: false` has no state extension and was probed at its defaults.
`skipped` lists bypassed and missing slots.

| Field | What it means | Target |
|---|---|---|
| `thd_pct` | Total harmonic distortion, H2..H9, % | Master 0.1-1, bus 0.5-3, single track 3-10. |
| `h` | H2..H9 in dBc, `h[0]` = H2 | -160 is the floor (absent). `null` means above Nyquist: that harmonic aliases instead. |
| `h2_h3_db` | H2 minus H3, dB | Above 0 is even-dominant, the "warm" signature. Below 0 is odd-dominant: harder, edgier. |
| `decay_db_per_order` | How fast the series falls, dB per order | 6 or more. A slow decay means high orders, which sound harsh. |
| `aliasing_floor_dbc` | Strongest bin that is not a harmonic or DC | -90 or lower. Probe at `freq_hz: 5000` to expose aliasing: harmonics past Nyquist fold back into the audible band. |
| `imd_pct` | SMPTE intermodulation (60 Hz + 7 kHz, 4:1), only with `imd: true` | Lower is cleaner. It rises when bass modulates the highs, which is muddy distortion. |
| `gain_db` | Output level at the probe frequency minus the input level | A saturator with auto-gain sits near 0. |
| `latency_samples` | Summed latency of the probed stages | Informational. |

**Probe at the level the chain really sees.** Distortion rises with level, so a
-12 dBFS probe of a bus that peaks at -3 understates it. `level_dbfs` sets the
tone's peak.

## Delivery: a normalized mixdown

`render_mixdown` can write a loudness-normalized file: pass `platform` (or
`normalize: {target_lufs, ceiling_dbtp}`). The mix is measured, gained to the
target, and true-peak limited at the ceiling. The project itself is untouched.

| `platform` | Target | Ceiling |
|---|---|---|
| `spotify`, `youtube`, `tidal` | -14 LUFS | -1 dBTP |
| `apple` | -16 LUFS | -1 dBTP |
| `amazon` | -14 LUFS | -2 dBTP |
| `deezer` | -15 LUFS | -1 dBTP |
| `club` | -8 LUFS | -0.3 dBTP |

The result's `normalize` block reports `achieved_lufs` and `achieved_dbtp`,
re-measured from the written file. If `achieved_lufs` sits clearly below the
target, reaching it would have taken more limiting than the ceiling allows. Do
not push harder here: the master needs its own glue and limiting first, and its
PSR (see `dynamics`) should stay at 8 or more.

## Cost

A measurement renders the slice offline — roughly what a bounce of the same
length costs. `meter_stems` renders each target in turn, so a 20-track project
is 20 full-length renders and can exceed the 5-minute wait; it then returns a
running job for `job_status` / `job_wait`.

`include_busses: true` adds each group/return bus. A bus and its member tracks
then both appear, describing the same audio before and after the bus chain —
they overlap, so **never add them together**.

Sub-tracks never get their own entry: the extra output ports of a multi-output
instrument are measured as part of their parent and listed in that entry's
`includes_track_ids`. A drum kit is therefore counted exactly once.

## Do not measure by bouncing

`render_mixdown` always bounces the whole song, and offset 0 in the file is the
earliest clip, not bar 1. Bounce for the user to listen to — never to work out
where something sits or how loud it is.
