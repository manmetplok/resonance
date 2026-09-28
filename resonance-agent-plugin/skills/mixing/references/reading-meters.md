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
| `correlation` | Stereo phase | Near +1 mono-identical, near 0 wide, **negative means anti-phase content a mono listener loses**. |
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

Both tools take `detail: ["spectrum"]`. Each named detail adds one object to
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
