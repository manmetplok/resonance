# Reverb algorithms: spec

Status: **built, 2026-10-06.** R0–R9 are on master: nine algorithms
(Classic, Plate, Room, Chamber, Hall, Ambience, Spring, Nonlinear, Shimmer),
the `resonance_metering::decay` harness, the `decay` meter detail, and the
skills (§9). What differs from the text below is recorded in §5.3. Open: a
live agent session that mixes a real project with the type table (R7's exit
check, needs the running app), and the by-hand Wayland editor tests.

## 1. Why

`resonance-reverb` has one algorithm. Every preset is a re-parameterisation of
the same tank, so "Vocal Plate", "Warm Hall" and "Snare Gated" are the same
room at different sizes. A mix engineer reaches for a *type* first (plate on
the vocal, a short room on the drums, a hall on the strings) and sets
parameters second. The agent skills cannot make that choice either, because
there is nothing to choose between.

### 1.1 What the current tank is (and gets wrong)

`src/dsp/chain.rs`: return EQ → pre-delay → 4-step Hadamard diffusion
(Signalsmith/Luff) → 8-line FDN with Householder feedback → stereo fold, plus a
parallel 12-tap early-reflection bank. It is a decent general-purpose
algorithm, and it stays as **Classic**, bit-identical. Its limits are what
this spec fixes, one algorithm at a time:

| # | Limit | Where | Consequence |
|---|---|---|---|
| L1 | Decay gain comes from one "typical loop" (`room_size_ms * 1.5`), not from each line's length | `ReverbDsp::set_decay` | The RT60 is only approximately the Decay knob, and short lines decay slower than long ones. The tail's spectrum colours as it decays. |
| L2 | One one-pole low-pass in the loop is the only frequency-dependent loss | `fdn.rs` `damping` | No control of low-frequency decay. Real halls and chambers hold their lows longer, plates hold their highs. "Damping" also moves the mid decay. |
| L3 | 8 lines spread over a 1×–2× range, lengths not mutually prime | `FdnBank::set_size` | Modal density is low for long decays: a long tail rings and "flutters" on sustained material. |
| L4 | Sine LFOs on the read taps | `modulation.rs` | Fine for chorus. Under 8 s+ tails the periodic detune is audible as a regular wobble. Lexicon-style random modulation hides it. |
| L5 | ER pattern is one fixed random set, scaled in time | `er.rs` | One "room shape" only. No small-room/large-room difference in the reflection pattern, only in its spacing. |
| L6 | No type-specific structure | — | No plate (immediately dense, no discrete ERs, bright decay), no spring (dispersive chirps), no gated/nonlinear envelope (the "Snare Gated" preset is a 0.3 s decay, not a gate), no shimmer (the "Shimmer Drone" preset has no pitch shifter). |
| L7 | Pre-delay and decay are in ms/s only | params | Tempo sync is done by hand in the skill (depth.md §2), and decay sync is not available at all. |

## 2. Research: what the algorithms are and what mixes use them for

### 2.1 The algorithm families

| Family | Structure | Character | Reference |
|---|---|---|---|
| **Plate** | Input allpass diffusers into a figure-8 "tank": two cross-coupled branches, each a modulated allpass, a delay, a damping filter and a second allpass, with the outputs tapped from many points of both branches | Dense from the first millisecond, no discrete early reflections, smooth, bright, slightly metallic. It sits *on* a source rather than placing it in a room. | Dattorro, "Effect Design Part 1", JAES 45(9), 1997. The topology is Griesinger's (Lexicon 224/480L). |
| **Room** | Shoebox early reflections (first and second order, image-source or tuned taps) + a short, dense FDN | Fast build (~10 ms), strong ERs, short decay (0.2–1.2 s). Places a source in a believable space. | Valhalla Room mode notes; Moorer, "About this reverberation business", 1979 (ER + late split) |
| **Chamber** | Like Room but denser ERs, more low-frequency decay, little or no modulation | The 1960s echo chamber: warm and dense, kind to vocals. | Valhalla Room/Vintage "Chamber" notes |
| **Hall** | Sparse, widely spread ERs (30–120 ms) feeding a large FDN with a slow build, random modulation, long bass decay, short treble decay | Large, slow-blooming, darker. Strings, pads, ballads, orchestral. | Jot & Chaigne, AES 1991 (FDN with per-line absorption); Lexicon Random Hall |
| **Ambience** | ER cluster with a very short dense decay (≤ 1 s), little audible tail | "Space without reverb". Depth and glue without a wash, so it suits a whole mix bus. | Valhalla / Lexicon "Ambience" |
| **Spring** | Cascades of dispersive allpasses (chirp), a feedback loop and a low-pass, with "drip" from the transient | Boingy, dispersive: guitars, surf, dub, lo-fi | Välimäki, Parker & Abel, parametric spring model (in "Fifty Years of Artificial Reverberation", IEEE TASLP 2012) |
| **Nonlinear / gated** | A dense tail shaped by an envelope that is not exponential: flat then cut (gated), rising (reverse) | The 1980s gated snare, reverse swells | AMS RMX16 "Nonlin"; Välimäki et al., dark-velvet-noise non-exponential reverb (2024) |
| **Shimmer** | A hall whose feedback loop contains a pitch shifter (+12 or +7) | Rising, organ-like halo: ambient, drones, post-rock | Eventide/Valhalla shimmer |

Two engine-level techniques apply across families:

- **Per-line absorption (Jot).** Each delay line of length *dᵢ* gets the gain
  that decays it by exactly 60 dB in T60 seconds,
  `gᵢ = 10^(−3·dᵢ / (T60·fs))`. Frequency-dependent decay comes from a
  shelving filter per line, designed for T60(low), T60(mid) and T60(high).
  This fixes L1 and L2 at the root. Refinements: Schlecht & Habets fit the
  shelf parameters to the target curve. Graphic-EQ absorption filters (Välimäki
  & Liski) are overkill for three bands.
- **Colourless feedback.** A lossless (orthogonal) matrix (Hadamard or
  Householder) with mutually prime line lengths spread over a wide range,
  enough lines for the decay (16 for halls), and slow random modulation of a
  few lines to break up the remaining modes. This fixes L3 and L4.

### 2.2 What a mix uses each one for

This table is what the skills encode (§9). Every number is a start, then
verified by measurement.

| Job | Type | Decay | Pre-delay | Notes |
|---|---|---|---|---|
| Lead vocal sheen | Plate | 1.2–2.5 s | 20–40 ms (tempo) | Return HPF ~300–600 Hz. Duck from the vocal. |
| Lead vocal, warm/intimate | Chamber | 1.0–1.8 s | 10–30 ms | Singer-songwriter, soul. |
| Snare | Plate (bright), Room (natural), Nonlinear (80s) | 0.8–1.8 s / 0.4–0.9 s / 0.25–0.5 s gate | 0–15 ms | Kick stays out. |
| Drum kit "in a room" | Room | 0.3–0.9 s | 0–10 ms | ER-forward (ER/tail toward −). Short decay keeps the groove. |
| Acoustic guitar, piano | Room or Chamber | 0.8–1.6 s | 10–20 ms | |
| Strings, pads, orchestral | Hall | 2–4 s (ballad up to 6) | 0–20 ms | LF decay ×1.2–1.5, HF ×0.4–0.6. Back layer. |
| Clean electric guitar | Spring | — | 0 ms | Or Plate for a hi-fi version. |
| Ambient beds, drones | Shimmer, Hall + Freeze | 6–20 s | 0–80 ms | Back layer, HPF 150–250 Hz. |
| Whole-mix glue, a dry mix | Ambience | 0.3–0.8 s | 0–10 ms | 3–10 % on a mix bus. Never long. |
| Bass, kick, sub | none | | | Low end stays dry and mono. |

Mix conventions the algorithms have to support:

- **Decay from tempo.** A rhythmic part's tail should fall well down before
  the next strong beat: T60 ≈ *n* × 60 / BPM, with *n* = 1–2 beats for busy
  material and 1–2 bars for ballads. This is L7, `decay_sync`.
- **Layers by type, not only by send.** Front = Ambience/Room (ERs, short),
  middle = Plate/Chamber, back = Hall (long, dark, slow build). The `spatial`
  skill's "one shared room" stays the default. Type-per-layer is the
  escalation when one room cannot place both front and back (depth.md §4
  already allows a second return).
- **Mastering adds no reverb.** Space is a mix decision. The only master-bus
  uses are (a) a very short Ambience at a few percent wet when the user asks
  for "glue" and the stems cannot be revisited, and (b) a tail held over an
  abrupt last chord, automated in. Both are measured (the `mastering` skill, §3c).

## 3. Scope

In:

- an `algorithm` choice and five mix algorithms (Plate, Room, Chamber, Hall,
  Ambience) on a shared engine with per-line absorption;
- three creative algorithms (Spring, Nonlinear, Shimmer) as a later phase;
- tempo-synced pre-delay and decay;
- a measurement harness that proves each algorithm's decay, density and
  colour by numbers rather than by ear;
- re-voiced and new factory presets;
- the skill updates (§9).

Out:

- Convolution. `resonance-ir` already does it. A "sampled hall" is an IR
  loaded there.
- Surround and Ambisonics. The plugin stays stereo.
- Any change to Classic's sound. Its goldens stay as they are.
- A new plugin crate. This is one plugin with more algorithms
  (plugin-audit-plan: no sprawl of single-trick plugins).

## 4. Design

### 4.1 Parameters

Appended after index 21, so existing indices and host automation are
unchanged. Every new parameter defaults to a no-op for Classic. With
`algorithm = Classic` the new parameters are ignored, and the editor greys
them out.

| Idx | Key | Name | Range / labels | Default | Used by |
|---|---|---|---|---|---|
| 22 | `algorithm` | Algorithm | `Classic`, `Plate`, `Room`, `Chamber`, `Hall`, `Ambience`, `Spring`, `Nonlinear`, `Shimmer` | `Classic` | all |
| 23 | `low_decay_mult` | Bass Decay | 0.25–4.0 ×, skewed | 1.0 | all but Classic, Spring, Nonlinear |
| 24 | `low_xover` | Bass Xover | 50–1000 Hz | 250 | same |
| 25 | `high_decay_mult` | Treble Decay | 0.05–1.0 × | 0.5 | same. The crossover is the existing `damping` (re-read as "the frequency above which decay is ×Treble Decay") |
| 26 | `predelay_sync` | Pre-delay Sync | `Off`, `1/128`, `1/64`, `1/32`, `1/16`, `1/8` | `Off` | all, Classic too. Overrides `predelay` while the host supplies tempo. |
| 27 | `decay_sync` | Decay Sync | `Off`, `1/4`, `1/2`, `1 bar`, `2 bars`, `4 bars` | `Off` | all but Nonlinear. Sets T60 to that length at the host tempo. |
| 28 | `tail_build` | Build | 0–1 | 0.5 | Hall, Shimmer: ER-to-late crossfade time (attack of the tail) |
| 29 | `shimmer_pitch` | Shimmer Pitch | `+12`, `+7`, `+5`, `-12`, `+19`, `+24` | `+12` | Shimmer |
| 30 | `shimmer_amount` | Shimmer | 0–1 | 0.3 | Shimmer: share of the loop that is pitch-shifted |
| 31 | `nl_shape` | Nonlin Shape | `Gated`, `Reverse`, `Flat` | `Gated` | Nonlinear |
| 32 | `nl_length` | Nonlin Length | 50–1000 ms | 300 | Nonlinear |
| 33 | `spring_tension` | Tension | 0–1 | 0.5 | Spring: chirp rate (dispersion) |
| 34 | `spring_drip` | Drip | 0–1 | 0.3 | Spring: transient boing |

The `algorithm` labels are fixed by this table. The `IntRange` max grows as
phases land (R3 adds `Plate` …), so an unbuilt algorithm is never selectable.
Labels are appended in the order shown even though they land out of order:
Plate (R3) is index 1 although Room (R4) is index 2.

`decay` keeps its 0.1–30 s range and means T60 at mid frequencies for every
algorithm. Ambience clamps it at 1.0 s, and Nonlinear ignores it in favour of
`nl_length`. `size` keeps meaning "bigger space", mapped per algorithm (for
example plate size scales the tank, room size the shoebox dimensions).

**Legacy state.** A project saved before this has no `algorithm` key and must
load as Classic. The default *is* Classic, so that holds for free.
`tests/legacy_state.rs` pins it.

### 4.2 Engine structure

```
src/dsp/
  chain.rs          # ReverbDsp: return EQ, pre-delay, duck/mix stay here
  algo/mod.rs       # enum Engine { Classic(..), Plate(..), Room(..), … }
  algo/classic.rs   # today's diffusion + FDN + ER, moved verbatim
  algo/plate.rs     # Dattorro tank
  algo/room.rs      # shoebox ER + FdnN<16>; Chamber is a voicing of it
  algo/hall.rs      # spread ER + FdnN<16> + build envelope
  algo/ambience.rs  # ER cluster + short FdnN<8>
  algo/spring.rs, nonlinear.rs, shimmer.rs   # R8
```

- **Enum dispatch, no `dyn`, no allocation on the audio thread.** Every engine
  is constructed in `initialize` at the session sample rate and lives for the
  instance. Memory is bounded: the 16-line hall at 96 kHz with the longest
  `size` is the largest, at about 2 MB. Total for all nine engines must stay
  under 8 MB at 96 kHz, asserted by a test on the allocation sizes.
- **Algorithm switch while running.** The outgoing engine stops receiving
  input and its tail is faded out linearly over 50 ms, while the incoming
  engine starts from silence. Both run only during the fade. The switch has no
  click (asserted the same way as `tests/glide.rs`), and a switch requested
  mid-fade is queued (newest wins), the pattern `er.rs` already uses.
- **Shared stages stay outside the engine**: return EQ, pre-delay, ER/tail
  balance, width, ducker, mix. Every algorithm gets them unchanged, and the
  skills' return-EQ and ducking keys keep working on every type.
  The ER/tail balance needs each engine to return `(er_l, er_r, late_l, late_r)`.
  Plate and Spring have no discrete ERs, so for them it weights their first
  50 ms (the onset) against the rest.
- **A switch while frozen is deferred** until Freeze is released, then fades
  as usual; switching immediately would fade the held tail into an engine
  that is frozen and empty.
- **Freeze** is per engine: loop gains to 1 and input muted (as today). Plate,
  Room, Chamber, Hall, Ambience and Shimmer support it. Nonlinear and Spring
  ignore it, and the editor greys it out.

### 4.3 Shared primitives (into `resonance-dsp`)

New module `resonance_dsp::reverb`, used by the plugin and available to
others (the granular delay's diffuser is a candidate):

| Primitive | What | Notes |
|---|---|---|
| `Fdn<const N: usize>` | N delay lines, orthogonal feedback (Householder for N=8, fast Hadamard for N=16), per-line `Absorption`, per-line modulated read | Line lengths: mutually prime, spread log-uniform over `[min, max]`, chosen once and scaled by size. Per-line gain from §2.1. |
| `Absorption` | One-pole high-shelf and one-pole low-shelf per line, designed from T60(low), T60(mid), T60(high) and the two crossovers for that line's length | Jot & Chaigne. First order keeps it cheap and phase-benign. |
| `Allpass` | Schroeder allpass, optional fractional modulated length, nestable | The plate's building block. |
| `SmoothRandom` | Interpolated random-walk modulator, seeded | Replaces sine LFOs in the new engines (L4). Deterministic per seed, `reset()` restores the seed. |
| `ShoeboxEr` | First- and second-order image-source taps for a box of dimensions `(x, y, z)` and source/listener positions, with frequency-dependent wall loss | Room/Chamber ERs. Computed on the main thread when `size` changes in steps, crossfaded like `er.rs`. |
| `DispersiveAllpass` | Cascade of first-order allpasses with high coefficients | Spring chirp. R8. |

### 4.4 The algorithms

**Plate (R3).** Dattorro's figure of eight: 4 input allpasses (diffusion),
two tank branches (modulated allpass → delay → damping/absorption → allpass →
delay), cross-fed, 7 output taps per side as in the paper, with delays scaled
from the paper's 29.76 kHz reference to the session rate. Absorption replaces
the paper's single damping filter, so plates get their characteristic long
treble (default `high_decay_mult` 0.8 for Plate). No ER bank. `size` scales
the tank delays 0.5×–1.5×.

**Room (R4).** `ShoeboxEr` (dimensions from `size`: 3×4×2.5 m up to
15×20×8 m), then a 16-line `Fdn` with short lines (5–60 ms) and light random
modulation. Fast build. `er_level` and `er_time` keep their meaning (ER level
and ER spacing scale).

**Chamber (R4).** A Room voicing: denser ER (second order weighted up),
`low_decay_mult` default 1.3, modulation depth default 0, slightly longer
lines (10–90 ms).

**Hall (R5).** Sparse ER spread over 30–120 ms (taps from `ShoeboxEr` with
large dimensions, thinned), 16-line `Fdn` with lines of 40–200 ms,
`SmoothRandom` modulation, and a `tail_build` envelope that crossfades ER into late
energy over 20–300 ms. Defaults: `low_decay_mult` 1.3, `high_decay_mult` 0.5.

**Ambience (R6).** A dense ER cluster (0–40 ms) plus an 8-line `Fdn`, decay
clamped to 0.1–1.0 s, no modulation. Designed to sit at a few percent on a mix
bus, so its late tail must be inaudible as a "reverb": its EDT, not its T60,
carries the space.

**Spring (R8).** Two parallel springs (L/R decorrelated). Each is a cascade of
`DispersiveAllpass` stages (count and coefficient from `spring_tension`) inside
a feedback loop with a low-pass at ~4.5 kHz, plus a transient-triggered
"drip" path (`spring_drip`). No ER. Mono-in.

**Nonlinear (R8).** A dense, colourless burst (a Room engine with very high
diffusion and decay held at a fixed long value) multiplied by an envelope:
`Gated` = flat for `nl_length` then a 10 ms release, `Reverse` = rising ramp
over `nl_length` then a cut, `Flat` = flat then a natural decay. The envelope
retriggers on transients of the input, using the ducker's detector type.

**Shimmer (R8).** Hall with a grain pitch shifter (two-tap crossfading delay,
or `resonance_dsp`'s granular engine) in one feedback path. `shimmer_amount`
sets how much of the loop goes through it. A low-pass after the shifter stops
octave-on-octave build-up turning into a whistle, and the loop gain is capped
so `+12` with Freeze cannot run away.

### 4.5 Tempo sync

`process` already receives `TempoInfo`. With `predelay_sync` ≠ `Off` and tempo
present, the pre-delay target is `240000 / bpm / k` ms for the label `1/k`
(a note value: a beat is a quarter note, so `1/64` is 31.25 ms at 120 BPM,
the vocal range §6's table uses), held under the 1 s pre-delay line. It
goes through the existing pre-delay crossfade, so a tempo change does not
click. `decay_sync` sets T60 = beats × 60 / bpm. Without tempo (offline host,
transport stopped with no tempo) both fall back to the ms/s knobs. The editor
shows the synced value as text next to the knob.

### 4.6 Editor

- Algorithm selector at the top of the editor (the shared choice-strip
  widget). Parameters not used by the algorithm are greyed out, never hidden,
  so the layout does not jump.
- The tank view becomes per-algorithm: an 8/16-line energy ring for FDN
  engines, the figure-of-eight for Plate, and a chirp trace for Spring.
- The impulse view keeps working: it already reads the wet RMS ring.
- Visual check through the plugin editor tests listed in CLAUDE.md
  (`editor_open`, `editor_size`).

## 5. Verification: numbers, because nobody here grades by ear

### 5.1 The harness (R0)

`resonance_metering::decay` (a runtime crate, not test support, so R9's
decay meter reuses it), used by the plugin tests and the bench:

| Metric | Method | Used to prove |
|---|---|---|
| T30 / T20 per octave band (125 Hz–8 kHz) | Octave-band filter, Schroeder backward integration, linear fit −5…−35 dB | Decay matches the knob, and Bass/Treble Decay do what they say |
| EDT | Fit 0…−10 dB | Ambience and Room feel short even at the same T60 |
| Echo density profile | Abel & Huang normalised echo density, 20 ms window | Plate dense at < 10 ms. Hall builds slowly. No sparse "flutter" late. |
| Modal peakiness | Late-tail (200 ms+) magnitude spectrum, 1/24-octave smoothing; max − median, dB | Colourlessness (L3) |
| Late IACC | Interaural cross-correlation of L/R over 80 ms+ | Stereo decorrelation at width 1 |
| Mono fold loss | Late L+R energy vs L and R energy | The wet return survives mono (the `spatial` skill's mono check) |
| Silence guard | Total impulse-response energy > −40 dB re a unit impulse, per scenario (a 2 s RMS > −60 dBFS sat only 0.5–3 dB above Classic's quietest settings) | No vacuous golden (memory: silent goldens) |

R0 also runs the harness on **Classic** and records its numbers in this spec,
as the baseline the new engines must beat on L1–L4.

**Classic baseline (R0, 48 kHz, unit impulse, 100 % wet, no pre-delay,
defaults otherwise; `cargo test -p resonance-reverb --test algorithms baseline -- --nocapture`):**

| size | decay | damping | mid T30 | err | EDT | 125 Hz | 1 kHz | 8 kHz | density 0.9 | peakiness | IACC |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0.2 | 0.5 s | 8 k | 0.47 | −7 % | 0.59 | 0.43 | 0.47 | 0.46 | 28 ms | 8.9 dB | 0.44 |
| 0.5 | 2 s | 8 k | 1.95 | −3 % | 1.01 | 1.95 | 1.91 | 0.83 | 62 ms | 7.0 dB | 0.15 |
| 0.9 | 2 s | 8 k | 2.10 | +5 % | 2.22 | 2.08 | 2.06 | 1.45 | 197 ms | 5.6 dB | 0.12 |
| 0.2 | 8 s | 8 k | 6.40 | −20 % | 4.17 | 7.63 | 5.78 | 0.55 | 28 ms | 10.3 dB | 0.07 |
| 0.5 | 8 s | 8 k | 7.19 | −10 % | 3.54 | 7.72 | 6.81 | 1.35 | 62 ms | 7.8 dB | 0.08 |
| 0.5 | 8 s | 20 k | 7.66 | −4 % | 4.71 | 7.73 | 7.52 | 3.11 | 62 ms | 7.0 dB | 0.05 |

(Exponentially decaying white noise reads 4.4 dB peakiness.) Findings: mid
decay is already within ±10 % except short sizes at long decays (−20 %), so
the new engines win on **band shape and colour**, not mid accuracy. Bass never
outlasts mid (L2). Treble dies early even with damping open (8 kHz holds 3.1 s
of an 8 s knob), so something besides the damping filter loses top: the
interpolated modulated reads or the diffusers. EDT does not follow the knob.
Density reaches 1.0 erratically (91–679 ms), so 0.9 is the threshold used in
§5.2. Mono fold is −3 dB everywhere (decorrelated, no cancellation). Classic
costs 15.8 µs per 128-frame block (0.6 % of the block budget), which is the
reference for the CPU budgets.

### 5.2 Acceptance per algorithm

| Algorithm | Decay accuracy (mid T30 vs knob) | Band decays | Echo density reaches 0.9 by | Peakiness | Late IACC |
|---|---|---|---|---|---|
| Plate | ±10 % | treble ≥ 0.7 × mid at default | 10 ms | ≤ Classic − 2 dB | ≤ 0.3 |
| Room | ±7 % | as set, ±15 % | 20 ms | ≤ Classic − 2 dB | ≤ 0.3 |
| Chamber | ±7 % | bass ≥ 1.2 × mid at default | 15 ms | ≤ Classic − 2 dB | ≤ 0.3 |
| Hall | ±7 % at 1–10 s | as set, ±15 % | 60–150 ms (slow build is the point) | ≤ Classic − 3 dB | ≤ 0.2 |
| Ambience | EDT ≤ 0.4 × T30 | — | 15 ms | — | ≤ 0.3 |
| Spring, Nonlinear, Shimmer | Nonlinear: gate length ±5 ms; others stability and silence-guard only | | | | |

Peakiness is compared with Classic at the same `size` and `decay` (the
baseline table's settings). **As built (R3–R5):** "Classic − N dB" alone is
unreachable for an engine whose decay matches the knob: Classic often reads
*below* exponentially decaying white noise of the same T60, because its real
tail outlasts the knob. Each cell asserts `≤ max(Classic − N, noise floor +
margin)` (margin 1.0 dB Plate, 1.5 Room/Chamber, 2.0 Hall, the floor measured
in-test from seeded decaying noise), and Hall's mean must sit ≥ 1 dB under
Classic's. Measured with `damping` at 20 kHz for Room/Chamber: an exact
treble decay tilts the late spectrum, which the metric counts as colour. The FDN engines get ±7 % because their per-line
gains are exact; Classic already manages ±10 % at most settings.

For every algorithm:

- **Stability fuzz.** 60 s of random parameter automation (every key, steps
  and ramps), noise and impulse input: no NaN or Inf, peak never above +24
  dBFS. Freeze held for 60 s: energy drift ≤ 0.1 dB.
- **Goldens.** One bit-exact impulse golden per algorithm in
  `tests/dsp_golden.rs`, plus one noise-burst scenario for the modulated ones.
  Every scenario carries its own silence guard and pins all params
  explicitly.
- **Classic unchanged.** The existing `dsp_golden`, `legacy_state` and
  `return_channel` goldens pass with no re-bless. A re-bless of any of them in
  this work is a bug.
- **Thread-count invariance.** The suite passes under
  `RESONANCE_RENDER_THREADS=8`.
- **Reset equals fresh** for every engine (`tests/reset.rs` pattern, including
  `SmoothRandom` seeds).
- **Switch is click-free.** Max sample-to-sample step during an algorithm
  switch under a sustained sine is ≤ the step without the switch + 1e-3.
- **CPU.** `benches/reverb_dsp.rs` gains one bench per algorithm at 48 kHz
  with 128-sample blocks. Budget relative to Classic (measured in R0): Plate,
  Room, Chamber and Ambience ≤ 1.5×; Hall and Shimmer ≤ 2.5×.

### 5.3 As built: other deviations (R3–R5)

- Plate: below a 0.87 s decay the tank shrinks so the loop is at most
  T60/1.2 (size is overridden there); the onset (`er_*`) is an 8-stage
  allpass cascade, `er_level` scales it, `er_time` stretches it.
- Room lines 6.4–75 ms (not 5–60: the shortest coloured), glide 0.04
  samples/sample (0.25 made block-rate size automation a staircase).
  Ambience at 0.3 s reports EDT/T30 0.44–0.51 (asserted only over the
  mix-bus range 0.5–1.0 s). Vocal Chamber's mid T30 is +5.6 % at its voicing
  (the 250 Hz shelf reaches into the 500 Hz band), asserted at ±10 %.
- Hall: `tail_build` spreads *when* energy enters the loop (taps on a build
  line at u·20 ms·15^build), so level and decay are independent of it.
  Density-0.9 is monotonic in build at size 0.5; at size 0.9, build 0 is
  lumpy (16 separate first returns), and the late-energy peak is the
  monotonic measure. Memory 2.6 MiB at 96 kHz. Wobble metric: spectral
  spread of a sustained sine (0.9 cents at defaults vs Classic's 9.8).
- Size-sweep click checks use step/peak and the second difference: sweeping
  drags room modes across a sine and legitimately changes its level.
- `Absorption` (R2) cannot give both shelves deep cuts with crossovers
  under ~2.5× apart: its mid compensation is capped at unity gain, so the mid
  decays short there. Tests stay inside the range.
- Freeze on the new engines is a 100 ms ramp, the same on engage and
  release (reversible mid-way): input and loop feed fade, the loop's loss
  falls to zero, and only then is the network exactly lossless. A hard gate
  clicked and the click was then held forever in the lossless loop. Plate's
  Freeze also mutes its onset (ER) path. Shared ramp:
  `src/dsp/algo/room/freeze.rs`.
- R9: the `decay` meter detail is read on the **track that sends** to a
  reverb return (its stem carries its sends' returns); a `{bus_id}` target
  hears only tracks routed into the bus, so a send-fed return reads silent.
- D2 mechanism: the plugin's `STATE_UPGRADE` hook (`params::upgrade_state`)
  inserts `algorithm` = Classic into any state whose non-empty `params` lacks
  the key; it runs on every load path (plugin, both CLAP bridge paths, preset
  bank/session). `Algorithm::DEFAULT` = Room is shared by the param and
  `ReverbDsp::new()`.
- Spring's onset (`er_*`) is the input's first pass through each spring (its
  own cascade); the recirculated passes are `late_*`. Same linear system,
  rounding-level difference; Spring now costs 16.6 µs per block.
- Editor minimum size is 800×700 (the R8 controls row pushed the return row
  off the strip at 720×680).
- CPU per 128-frame block at 48 kHz: Classic 15.6–16 µs, Plate 6.0, Room 16.9,
  Chamber 16.9, Hall 19.1, Ambience 7.9.

## 6. Presets

Factory presets keep their ids and names. Decision **D1** is whether existing
ones move onto the new algorithms:

| Preset | Today | Proposed algorithm |
|---|---|---|
| Tight Room, Snare Tight | Classic | Room |
| Vocal Plate, Snare Plate | Classic | Plate |
| Warm Hall, Cathedral | Classic | Hall |
| Snare Ambient | Classic | Room |
| Snare Gated | Classic | Nonlinear (R8; Classic until then) |
| Shimmer Drone | Classic | Shimmer (R8) |
| Ambient Bloom | Classic | Hall |

New presets, each with full `meta` (category, instrument, character, genres)
so `presets_search` finds them:

| Name | Algorithm | For |
|---|---|---|
| Vocal Chamber | Chamber | warm lead vocal |
| Drum Room | Room | kit ambience, ER-forward |
| String Hall | Hall | strings, pads, orchestral |
| Mix Glue | Ambience | a few percent on a mix bus |
| Short Ambience | Ambience | front-layer room |
| Bright Plate | Plate | snare and percussion sheen |
| Surf Spring | Spring | clean guitar (R8) |
| 80s Gate | Nonlinear | snare (R8) |

## 7. Phases

Each phase is a vertical slice: DSP, params, editor, tests and presets
together. Nothing is half-wired between phases.

| Phase | Delivers | Exit |
|---|---|---|
| **R0** | `resonance_metering::decay` harness; Classic baseline numbers recorded in §5; per-algorithm bench scaffold | Harness unit-tested on synthetic exponential decays (known T60 recovered to ±2 %). Baseline in this doc. |
| **R1** | params 22-27: `algorithm` (Classic only), the three decay multipliers (inert until R3; indices must stay contiguous), engine enum, switch crossfade, preallocation, shared-stage refactor; `predelay_sync`, `decay_sync` (both apply to Classic) | Classic goldens untouched; legacy state loads as Classic; sync tests at 60/120/170 BPM |
| **R2** | `resonance_dsp::reverb` primitives: `Fdn<N>`, `Absorption`, `Allpass`, `SmoothRandom`, `ShoeboxEr` | Each primitive tested in `resonance-dsp/tests/`: `Fdn` T30 per band ±5 % of the design, lossless matrix check, `Absorption` matches the target T60 at three frequencies |
| **R3** | Plate + `low_decay_mult` / `low_xover` / `high_decay_mult` | §5.2 row; goldens; Vocal Plate and Snare Plate re-voiced (if D1); Bright Plate |
| **R4** | Room, Chamber | §5.2 rows; Drum Room, Vocal Chamber, Tight Room re-voiced |
| **R5** | Hall + `tail_build` | §5.2 row; String Hall; Warm Hall, Cathedral, Ambient Bloom re-voiced |
| **R6** | Ambience | §5.2 row; Mix Glue, Short Ambience |
| **R7** | Skills (§9) land; `resonance-studio` reference updated | `agent_plugin_lockstep` and `skill_keys` green; one live agent session that mixes a project end to end using the type table, with its moves reported |
| **R8** | Spring, Nonlinear, Shimmer + their params and presets | §5.2 rows; Snare Gated and Shimmer Drone re-voiced; skills' creative rows land |
| **R9** *(optional)* | `meter.measure` detail `decay`: EDT and T30 of a target (a return) over a range that ends in silence | Control-API vertical slice (method + handler + MCP tool together). Lets the skills check "the room's tail clears before the downbeat" by number. |

R3 comes first because a plate is the single most-requested mix reverb (vocal
and snare) and it shares the least with the FDN engines. That makes it a good
test of the engine split.

## 8. Decisions

| # | Question | Recommendation |
|---|---|---|
| D1 | Re-voice existing factory presets onto the new algorithms (an audible change to presets that projects reference by id)? | **Yes**, per the §6 table. No real users yet (memory), and "Vocal Plate" that is not a plate is a misleading name. A project that loaded a preset keeps its *parameter values*, which include no `algorithm` key, so it stays Classic and sounds the same. |
| D2 | Default algorithm for a newly inserted reverb | **Classic until R7**, then **Room**. Changing the param default also changes what a legacy state (no key) loads as, so R7 must make the legacy loader set `Classic` explicitly when the key is absent, with `legacy_state.rs` proving it. |
| D3 | Expose `algorithm` as a dedicated control-API field or just as a param? | **Just a param.** `track_set_plugin_param` already takes choice labels, and lockstep checks them. No protocol bump. |
| D4 | Build R9 (decay meter)? | Yes, after R7. Without it the skills set decay from tempo arithmetic and can verify only DRR, not the tail's length. |

## 9. Skill changes (as landed, R7 with the R8 creative rows)

The drafted text that stood here was written before the engines existed; what
landed is in the skills themselves, revised to the built algorithms:

- `resonance-agent-plugin/skills/spatial/references/depth.md`: §1 (the shared
  room: load a preset or set `algorithm` first, never the default; presets
  store an insert mix, so `mix` 1.0 after loading), §1b "Pick the room by job"
  (type and starting preset per job, all nine presets' jobs, what each type
  reads and ignores), §2 (`predelay_sync`/`decay_sync`), §4 (second returns by
  type: back/wash, front, snare, guitar) and §7 "Verify the tail" (the R9
  `decay` detail on a sending track: `t30_seconds` against the intended decay,
  `stop_seconds` + `tail_20db_seconds` against the next downbeat, `bands`
  against the bass multiplier). The spatial `SKILL.md` step 5 points there.
- `mixing/references/roles.md`: a Room type table per role.
- `mastering/SKILL.md` §3c "Space on the master (rarely)": the rule plus the
  two exceptions (Ambience/Mix Glue before the chain, a Hall tail automated in
  over an abrupt ending).
- Outside the repo: `~/.claude/skills/resonance-studio` (the Reverb line of
  references/resonance-reference.md), `singer-songwriter`,
  `industrial-post-metal` and `resonance-synth-design` swap preset-only advice
  for type + preset and point at depth.md §1b.

Not carried over from the draft: Freeze on the master tail (a frozen tail
never ends and a bounce runs only 2 s past the last clip, so it would be cut;
the skill uses a 2-3 s Hall checked by `dynamic_range_db` instead), and "the
`depth` ordering unchanged" as a master-glue check (stems are measured before
the master chain, so a master insert cannot move it).

## 10. References

- J. Dattorro, "Effect Design Part 1: Reverberator and Other Filters", JAES 45(9):660–684, 1997.
- J.-M. Jot, A. Chaigne, "Digital Delay Networks for Designing Artificial Reverberators", AES 90th Convention, 1991.
- V. Välimäki, J. Parker, L. Savioja, J. O. Smith, J. S. Abel, "Fifty Years of Artificial Reverberation", IEEE TASLP 20(5), 2012 — https://aaltodoc.aalto.fi/bitstreams/97ed04a8-cb88-461f-b1a3-e72da5129256/download
- S. J. Schlecht, E. A. P. Habets, "Accurate reverberation time control in feedback delay networks", DAFx 2017; improved T60 control: https://www.researchgate.net/publication/335756510_Improved_Reverberation_Time_Control_For_Feedback_Delay_Networks
- L. Dahl, J.-M. Jot, "A Reverberator Based on Absorbent All-Pass Filters", DAFx 2000 — https://ccrma.stanford.edu/~lukedahl/pdfs/Dahl,Jot-AbsorbentAllPassReverbs-Dafx2000.pdf
- J. S. Abel, P. Huang, "A Simple, Robust Measure of Reverberation Echo Density", AES 121st Convention, 2006.
- Non-exponential reverb with dark velvet noise (2024): https://arxiv.org/abs/2403.20090
- G. Luff, "Let's Write a Reverb" (Signalsmith, ADC 2021): the Classic engine's design.
- Valhalla DSP on modes and their uses: https://valhalladsp.com/2011/05/03/valhallaroom-the-reverb-modes/, https://valhalladsp.com/2023/02/10/valhallavintageverb-the-modes/, https://valhalladsp.com/2014/01/18/naming-reverb-algorithms/
- Dattorro plate implementations for cross-checking: https://valleyaudio.github.io/rack/plateau/
- Existing in-repo: `warmth-width-depth.md` §6.4 (return EQ, ducking, ER/tail), the `spatial` skill's depth.md.
