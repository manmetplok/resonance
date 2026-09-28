# Warmth, width and depth — mix character for agents and humans

Status: **W0–W12 built and merged 2026-09-28**, decisions in §10.
Deviations found while building: the PurestWarm one-polarity curve is not
even-only, so `Warm` is a strictly even curve instead. The delay "Haas — Safe"
preset was dropped, because Haas lives in `resonance-stereo`. De-harsh
defaults are Q 24 / selectivity 5 (`docs/design/deharsh-resonance-suppressor.md`).
Research-backed
spec. §1–§4 are the research digest (vocabulary, measurable proxies, the
field, the procedure), §5 is what exists, §6–§8 are the proposal, §9 is
the build order. Land it as vertical slices in the usual
way: every control method ships with its wire type, handler, MCP tool and
tests together, and every skill change is re-read against
`agent_plugin_lockstep.rs`.

## 0. Why

The request is "add warmth to a mix", and then the same for stereo image
and depth. Two separate problems sit underneath it:

1. **The DSP to do it is thin.** Resonance has exactly one warmth device,
   the mastering saturator (`sat_*`): tanh or cubic with ADAA, a Tube→Tape
   blend, and a fixed tape voicing. It exists only on the master chain.
   There is no saturator for tracks or busses. There is also no M/S EQ, no
   per-band width, no tilt band, no clipper, no dynamic EQ, no wet-signal
   filters on the reverb, and no mono-safe widener.
2. **An agent cannot hear, and warmth/width/depth are the least measurable
   parts of a mix.** Today's meters give four coarse `bands`, a whole-range
   `correlation` scalar, and `mono_penalty_db`. "Sterile, wants density" in
   the mastering skill has no number behind it, so an agent that adds
   saturation cannot tell whether it made things warmer or just louder.

The second problem is the bigger one. **Analysis comes before DSP** in the
build order (§9), because every DSP addition is only as useful to an agent
as the measurement that verifies it.

### Goals

- Agents can make a mix measurably warmer, wider (mono-safely) and deeper,
  and can verify each move with a number, at matched loudness.
- One character plugin that works on tracks and busses, plus targeted
  extensions to `eq`, `reverb` and `mastering`. No sprawl of single-trick
  plugins.
- Craft knowledge lives in the skills; plugins and meters carry only what
  has to be computed (§8 draws the line).

### Non-goals

- Circuit-exact emulation of named hardware. We take the *behaviours*
  (asymmetry, LF-weighted saturation, head bump, HF self-erasure) and give
  them generic names. There are no brand names in UI or presets.
- An ML auto-mastering assistant ("LANDR"). The existing assistant's
  deterministic LTAS-vs-target approach is the right shape. It just needs
  exposing (§7.4).
- Immersive, binaural or HRTF panning (dearVR, Precedence). Stereo only.
- Changing the sound of existing projects. All new stages default off or
  to bit-transparent (the plugin-audit dual-surface rule applies).

## 1. Vocabulary: what the words mean technically

The agent needs these definitions in its skills. They are the bridge from
a user's adjective to a parameter.

| Word | Signal-level meaning | Main levers |
|---|---|---|
| **Warm** | (a) Low-order, even-dominant harmonics (H2 ≥ H3, series falling ≥ 6 dB/order) at low level. (b) Spectral tilt slightly steeper than the genre norm: a little more 100–400 Hz, a little less 2–5 kHz and 10 kHz+. (c) Softened transients, crest factor down 0.5–2 dB. | Asymmetric saturation, tape/transformer stage, tilt/shelves, de-harsh, glue comp |
| **Harsh / cold** | Energy peaks in 2–5 kHz; odd, slowly decaying harmonics; inharmonic aliasing products | Dynamic cut 2–5 kHz, softer shaper knee, oversampling/ADAA |
| **Muddy** | 200–500 Hz build-up, often from many wide sources | Cut low mids, especially on the sides (M/S) |
| **Wide** | Side energy relative to mid (S/M ratio) with correlation still healthy; decorrelated L/R above ~150 Hz | Double tracks, pan, M/S side gain, decorrelation, stereo reverb |
| **Mono-safe** | Mono fold-down loses little level and has no deep combs; low band correlation ≈ +1 | Mono-maker below 100–150 Hz, avoid static Haas |
| **Deep** | Clear *contrast* between front, middle and back layers in DRR, pre-delay, HF content, transient sharpness and width | Send levels, pre-delay, return EQ, per-layer darkening, ER vs tail |
| **Upfront** | Dry, bright, transient-intact, long pre-delay (≥ 30 ms) or delay instead of reverb, reverb ducked | Send down, pre-delay up, duck the return |

Two rules from the research drive the whole spec:

- **Loudness confounds everything.** A saturator raises RMS, and louder
  reads as warmer and better. Every before/after judgement, whether human
  or agent, must happen at matched integrated LUFS (±0.5 LU). Chow Tape,
  TDR SlickEQ, Decapitator and Gullfoss all build compensation in for this
  reason.
- **Depth is relative.** Joe Lambert and Sonible both frame depth as
  contrast between layers, not as absolute settings. An agent verifies
  *ordering* (lead DRR > backing DRR > pad DRR), not a target number.

## 2. Measurable proxies

These are the numbers an agent should be able to read. §7 makes them
available. Thresholds are practitioner heuristics unless marked with a
source in §12.

### 2.1 Warmth / tone

| Proxy | Definition | "Warmer" means |
|---|---|---|
| Spectral tilt | Slope of the 1/3-oct LTAS regression over 100 Hz–10 kHz, in dB/oct | More negative, e.g. −4.0 → −4.8. Commercial pop averages about −4.5 to −5 dB/oct (Pestana et al., 772 singles) |
| Spectral centroid | Power-weighted mean frequency | Falls about 5–15% at matched loudness; falling further means "dull" |
| Low-mid / presence ratio | E(150–500 Hz) / E(2–5 kHz), in dB | Rises about 1–2 dB |
| Presence peakiness | Spectral crest (max/mean) inside 2–5 kHz, 1/6-oct | Falls; resonances tamed |
| Air ratio | E(8–16 kHz) / total | Slightly down |
| Crest / PSR | Existing `crest_db`; new `psr_db` = true peak − max short-term LUFS | Down 0.5–2 dB. Stop above 2, and keep PSR ≥ 8 dB |
| Harmonic signature | THD, H2/H3, per-order decay, aliasing floor of a plugin *chain*, from a probe tone (§7.3) | H2/H3 > 0 dB, decay ≥ 6 dB/order, aliasing ≤ −90 dBc |

THD targets for a character stage: **master 0.1–1 %** (−60 to −40 dBc),
**bus 0.5–3 %**, **single track 3–10 %**.

### 2.2 Width / mono safety

| Proxy | Definition | Healthy |
|---|---|---|
| Correlation (integrated) | E[LR]/√(E[L²]E[R²]) | Whole mix +0.2…+0.8, typical 0.3–0.7 |
| Correlation (windowed) | 400 ms windows | Warn if < +0.3 for > 10% of windows; fail if < −0.1 for > 1 s |
| Per-band correlation | 8 bands (or 1/3-oct) | < 150 Hz ≥ +0.9; 150 Hz–1 kHz ≥ +0.5; > 1 kHz ≥ 0 |
| S/M ratio per band | 10·log10(E[S²]/E[M²]) | < 150 Hz ≤ −20 dB; mids −12…−4; highs −8…−2; whole mix −10…−4 |
| Mono fold loss | LUFS((L+R)/2) − LUFS(stereo), per track and per band | Mix ≤ 3 dB; per track ≤ 6 dB and no deep combs |
| Haas detector | Peak of the L/R cross-correlation at lag 1–35 ms | Normalized peak > 0.5 flags a comb risk at f = (2k+1)/(2τ) |
| Balance | 10·log10(E_L/E_R) | Mix within ±1 dB; per section within ±1.5 dB |

With equal L/R energy the two numbers map onto each other:
r = (1−ρ)/(1+ρ), where ρ = S/M power. S/M in dB is the more linear
"width" number, so the skills should reason in S/M and treat correlation
as the fault detector.

Hard-panned mono makes r = 0/0. The meter must report this explicitly
(`one_sided: true` plus the balance) instead of 0 or +1. Meters in the
field disagree on this case.

### 2.3 Depth

| Proxy | Definition | Healthy |
|---|---|---|
| DRR per source | Dry-path energy vs the energy that source contributes to the reverb returns, from separate renders | *Ordering* front > middle > back. Rough targets front ≥ +10 dB, middle +3…+8, back ≤ 0 |
| HF tilt per layer | E(6–16 kHz)/E(1–4 kHz) per track, grouped by layer | Decreases front → back |
| Ducking check | Return energy during vs after vocal phrases | "During" ≥ 3–6 dB below "after" when the return is ducked |

## 3. How the field does it (research digest)

This section is condensed. Sources are in §12. The "take" column is what
we adopt.

### 3.1 Warmth and saturation

| Tool | What it actually does | Take |
|---|---|---|
| **Chow Tape Model** (GPL, open) | Jiles-Atherton hysteresis ODE with drive/sat/bias mapped to M_s, a, c. Selectable solvers (RK2/RK4/NR). Oversampled 2× by default. Separate loss filters (spacing `e^-kd`, gap `sinc`, thickness), head bump, wow/flutter | Reference for a physically based tape stage. The hysteresis is stateful, so ADAA doesn't apply and it needs oversampling |
| **Airwindows** Console7 / Density / PurestWarm / ToTape6 (MIT, open) | Console: `sin` per channel and `asin` on the bus. Density: iterated `sin` with a negative "anti-sat" mode. **PurestWarm: saturates one polarity only, which gives pure even harmonics.** ToTape6: soften, servo-damped head bump, flutter bypassed at 0 | Cheap, well-understood curves. The PurestWarm idea is the simplest way to get even-dominant harmonics |
| **Oxford Inflator** (clone: RCInflator2 JSFX) | Odd polynomial `Ay+By²+Cy³−D(y²−2y³+y⁴)` with a Curve control, an Effect mix, and an optional 3-band split at 240 Hz/2.4 kHz | A "loudness without limiting" density stage. Odd-only, so it is not a warmth tool, but it is a useful master option |
| **Decapitator / Radiator** | Five styles (preamp, EMI, Neve, triode = even, pentode = odd). Drive, low/high cut, **Tone tilt**, mix, **auto-gain**, Punish | The control set is right: drive + style + tone tilt + mix + auto-gain |
| **Saturn 2** | Up to 6 bands, style per band, drive/feedback/**dynamics**/tone/mix. The envelope follower can modulate drive | Multiband drive: saturate lows and mids, leave the highs clean. Program-dependent drive |
| **Klanghelm IVGI/SDRR** | ASYM MIX (more asymmetric means more transparent and less compression) and **RESPONSE** (saturation focused on lows or highs), with level-dependent fluctuation | The "Response" tilt of the *drive* is exactly the transformer and tape behaviour. Cheap to build as pre/de-emphasis around the shaper |
| **Kazrog True Iron** | Six measured transformers. Distortion is frequency-dependent (flux ∝ V/f, so bass saturates first), has hysteresis, and has an inherent sub-sonic HPF with a small resonance | Transformer mode = LF-weighted drive + sub-sonic HPF + gentle HF resonance |
| **Black Box HG-2** | Pentode→triode stages, a parallel "saturation" path with frequency select, density, air shelf | Parallel saturation with a focus band |
| **Tape machines** (J37, Kramer, Studer/ATR, Softube Tape, VTM) | Speed (bump frequency ∝ speed: roughly 50–60 Hz at 15 ips, 100–120 Hz at 30 ips), bias (under = brighter/dirtier, over = darker/cleaner), formula, wow/flutter, noise, crosstalk | Tape mode knobs: speed (moves bump and HF loss), bias, flutter (default 0 = bypass) |
| **Pultec EQP-1A** | LF boost and cut on one switch through different capacitors. Doing both gives a lift around 60–80 Hz plus a dip around 200–300 Hz | An EQ band type "LF lift + mud dip" in one knob |
| **Maag EQ4 Air** | Very broad boost-only HF shelf with corners up to 40 kHz, so only the gentle part of the slope is audible | An "air" band type: a shelf corner above the audible range |
| **TDR SlickEQ / Nova** | Loudness-compensated auto-gain, stateful output saturation stages, M/S. Nova: parallel dynamic EQ | Auto-gain on EQ moves; dynamic EQ bands |
| **soothe2** | Many narrow, time-varying cuts on detected resonances (depth, sharpness, selectivity, soft/hard, M/S, delta monitoring) | The de-harsh stage (§6.3). A resonance suppressor restricted to a band |
| **Gullfoss / Ozone Stabilizer / smart:EQ** | Continuous perceptual auto-EQ toward a target | Not a DSP goal. The *static* version (measure LTAS, compare to target, set shelves) is what our assistant already does |
| **Ozone** Exciter / Imager / Low End Focus / Tonal Balance Control / Master Assistant | Multiband exciter with modes (Warm, Tube, Triode, Tape…). Four-band imager with Stereoize I (Haas) and II (velvet-noise, pure side). **Genre target bands from many tracks, plus a custom target from reference files.** Assistant = target loudness + genre + allowed modules | Target *bands* instead of a single curve. Reference-derived targets. "Allowed modules" is a good assistant API shape |
| **SSL G / Glue, Shadow Hills, Fairchild, Manley Vari-Mu, Kotelnikov** | Bus glue norms: 2:1–4:1, attack 10–30 ms, auto release, 1–3 dB GR. Vari-mu 1.5:1. Kotelnikov's parallel peak+RMS detectors | Our compressor already covers this. Add a program-dependent auto release (§6.4) |
| **Pro-L 2, StandardCLIP/KClip** | Limiter styles, true-peak detection, 4–32× oversampling. **Clip 1–3 dB of peaks before the limiter** so the limiter pumps less | A clipper stage before the limiter in mastering (§6.3) |

Guidance on aliasing: soft saturation needs 2× oversampling or ADAA;
heavy drive needs 4×; clippers need 8× or more. ADAA gets near-4× quality
at 1× cost for memoryless shapers only, not for hysteresis.

### 3.2 Width

| Technique | DSP | Mono risk | Take |
|---|---|---|---|
| Double-tracking | Two performances | None (natural decorrelation) | The best width. Skill guidance, no DSP |
| Pan / LCR | Resonance uses a *stereo balance* law, centre = unity, so hard-panning a source drops its perceived level about 3 dB | Hard-panned parts drop 6 dB in mono vs centre | Skill must compensate the fader about 3 dB after big pan moves |
| M/S side gain | S × w | S cancels in mono, so width from S gain alone vanishes in mono | Have it: `img_width`, mastering only |
| Haas | Delay one side by 5–30 ms | Deep combs, first null at 1000/(2·d_ms) Hz | Offer only with a level offset and a low-band exclude, labelled as a risk |
| Micro-shift (H910/MicroShift) | ±8–10 cents detune + 10/15–20 ms delays per side, with a Focus band | Moving combs, better than Haas | Mode in the width plugin (§6.2) |
| All-pass decorrelation (KERN) | 30–50 ERB-spaced all-passes, different per side | < 1–2 dB mono ripple; smears transients | Mode in the width plugin |
| Velvet-noise decorrelator (DAFx-17/18; Ozone Stereoize II) | Sparse ±1 FIR that generates **pure side** from mid | **Mono sum unchanged** | The primary "widen a mono source" mode. The mono-invariance is testable bit-exactly |
| Mono-maker / elliptical EQ (bx_control) | Side content below f_c moved into mid (mix: 100–200 Hz; vinyl: 30–50 Hz, 6–12 dB/oct) | Improves mono | Have it as `img_side_hpf_*`, but only on the master |
| Shuffler (Blumlein, Waves S1) | Widens *bass* width 1–3× below f | Mono-compatible | Low priority; acoustic material only |
| Per-band width (Ozone Imager) | Width per multiband band | — | Mastering imager can reuse the multiband crossovers |

### 3.3 Depth

The cues, strongest first: **level** (direct −6 dB per doubling of
distance), **DRR** (the most stable distance cue), **pre-delay** (in
pop/rock, long means close and short means far; orchestral practice
differs), **early reflections** (they carry position while the tail
carries room size, so layer depth with a shared tail plus per-layer ER),
**HF loss** (real air absorption is small, about 1.6 dB at 10 kHz over
10 m, so mix darkening is an exaggerated convention), **transient
softness**, and **width** (distant = narrower).

The standard production recipe:

- **Three layers.** Front: lead vocal, kick, snare, bass. Middle: guitars,
  keys, overheads, backing vocals. Back: pads, strings, FX.
- **Reverb on sends**, a shared room.
- **Return EQ.** Abbey Road trick: HPF about 600 Hz, LPF about 10 kHz,
  12–18 dB/oct, *before* the reverb.
- **Duck the return from the dry lead.** Ratio 4–8:1, attack 10–30 ms,
  release 100–300 ms.
- **Delay throws** on phrase ends instead of constant long reverb.
- **Pre-delay** 20–40 ms for vocals, tempo-derived (60000/BPM/8 = a 1/32
  note).

Tools built around distance: TDR Proximity, Sound Particles Air and
Airwindows Distance. Each one bundles gain, air absorption, proximity
effect and ERs behind a single **distance** control. That macro is the
idea worth taking (§6.5).

Clarity supports all three: frequency slotting (HPF guitars around
140–150 Hz against bass), M/S cut of 250–500 Hz on wide elements,
kick→bass sidechain, and spectral ducking (Trackspacer: a 32-band inverse
of the key spectrum; Neutron Unmask).

### 3.4 Mastering chain and targets

Standard order: gain stage → corrective EQ → M/S EQ → multiband (only if
needed) → glue → saturation/exciter → imaging above 200–300 Hz → clipper
(1–3 dB) → limiter → dither (only when reducing word length).

Resonance's mastering chain order matches, minus the M/S EQ and the
clipper.

Targets:

| Platform / measure | Value |
|---|---|
| Spotify | −14 LUFS-I; −2 dBTP if the master is louder than −14 |
| Apple Music | −16 LUFS-I |
| YouTube, Tidal, Amazon | −14 LUFS-I |
| Deezer | −15 LUFS-I |
| Club masters | −9 to −6 LUFS-I |
| PLR | 8–12 dB |
| PSR | ≥ 8 dB in the loudest section |
| Tonal slope | About 4.5–5 dB/oct |

Recent reports claim Spotify's normal-mode behaviour changed. The −19
"Quiet" figure is disputed. Neither changes a −14 / −1 dBTP recommendation.

## 4. The "make it warmer" procedure (what the skill encodes)

Condensed from the research. This is the canonical order an agent follows;
every step is measured at matched loudness.

1. **Baseline.** `meter_measure` on master (render) plus the new spectrum
   and stereo fields. Record tilt, centroid, LM/presence ratio, crest,
   PSR, per-band correlation, and LUFS.
2. **Fix harshness before adding anything.** Treat a 2–5 kHz peak or a
   high presence peakiness as a harshness problem. Apply a dynamic or
   static cut of −1 to −3 dB, per bus where the measurements point.
3. **Warmth on busses, not the master.** Put the character plugin on the
   drum, bass, music and vocal busses at bus THD targets, mix 20–50%, with
   even-dominant modes. Summing several lightly saturated busses gives
   cohesion without master-level IMD.
4. **Tone.** Gentle top shelf of −0.5 to −1.5 dB at 10–12 kHz, or tape HF
   loss. LF weight via LF-lift + mud-dip (+1…+2 dB at 60–100 Hz), or a
   broad +0.5…+1 dB at 150–300 Hz if the low mids are not already muddy.
5. **Glue.** 2:1, attack 10–30 ms, auto release, ≤ 2 dB GR.
6. **Master character, only if still sterile.** Tape or transformer mode
   at master THD targets, or parallel at 10–30%. Place it before the
   clipper and limiter, never after the limiter.
7. **Re-verify.** Compare against the baseline at matched LUFS. Accept only
   if tilt moved in the warm direction and nothing else got worse:
   - PSR still ≥ 8
   - no new aliasing
   - correlation and mono loss no worse
   - true peak ≤ −1 dBTP

   Report each move as *stage → number that justified it → cost*, the way
   the mastering skill already does.

The equivalent procedures for width (mono the lows → pan → doubles →
decorrelate mono sources above 150 Hz → M/S side shelf → check mono fold)
and depth (assign layers → shared room send → per-layer send, pre-delay
and HF → duck the lead's return → verify DRR ordering) go in the skill in
the same form.

## 5. What Resonance already has

This was verified in code. Paths are relative to the repo root.

| Area | Where | Facts that matter here |
|---|---|---|
| Mastering chain | `plugins/resonance-mastering/src/chain.rs` | Fixed order: corrective EQ → glue → **saturator** → tonal EQ → multiband → **imager** → limiter → dither. Every stage defaults OFF. Linear-phase EQ stages, 4 bands each (`{prefix}_b{n}_*`) |
| Saturator | `stages/saturator.rs`, `params/saturator.rs` | `sat_drive` 0–18 dB, `sat_character` Tube→Tape (asymmetry adds H2), `sat_mix`, `sat_shaper` Smooth tanh / Gritty cubic. 1st-order **ADAA**, no oversampling. Wet path: HF shelf cut → shaper → DC block → LF shelf → peak-normalize. Tape voicing is fixed (plugin-audit.md) |
| Imager | `stages/imager.rs` | `img_width` 0–2 (global side gain), `img_side_hpf_on/_freq` 20–400 Hz (mono-maker). **No per-band width** |
| Limiter | `stages/limiter.rs` | True-peak via 4× polyphase, 5 ms lookahead, `lim_ceiling`, `lim_release`. No clipper before it |
| Assistant | `plugins/resonance-mastering/src/assistant/` | 10 s capture → 1/6-oct LTAS → genre curve (`targets.rs`) or **reference track** (`reference.rs`) → `decide.rs` suggests trim/shelves/glue/imager/limiter with rationale. **GUI-only**; genre and reference are not persisted |
| EQ | `plugins/resonance-eq` | 8-band RBJ biquad; Bell/shelves/cuts; slope 12/24/48. **No M/S, no dynamic, no tilt, no linear phase.** Preset "Bass — Warm" |
| Compressor | `plugins/resonance-compressor` | Peak/RMS blend detector, `mix` (parallel), `sc_hpf_*`, `auto_makeup`, **external sidechain key**. No auto/program-dependent release |
| Reverb | `plugins/resonance-reverb` | `predelay` 0–250 ms, `er_level`, `er_time`, `size`, `decay`, `damping`, `diffusion`, `mod_*`, `width` 0–1, `mix`, `freeze`. **No wet HPF/LPF, no ducking** |
| Delay | `plugins/resonance-delay` | Sync, `stereo_offset`, `hi_cut`/`lo_cut`, `drive`, **self-ducking** (`duck_*`). Haas is possible only by hand |
| Wavetable FX | `plugins/resonance-wavetable/src/dsp/effects.rs` | `dist_mode` Soft/**Tube (biased, even)**/Fold/Hard/Crush with **`dist_oversample` Off/2×/4×** (the only user of the shared oversampler). BBD chorus modes |
| DSP primitives | `resonance-dsp` | `Biquad`, `eq::BandType`, `dynamics::*`, `Oversampler` (IIR half-band 1/2/4×, no fixed latency), `tanh_fast` (the only shaper), `DcBlocker`, `DelayLine`, `FftConvolver`, pan laws. **No tape, transformer, clipper, exciter, M/S helper, all-pass cascade or decorrelator** |
| Metering (wire) | `resonance-control/src/methods/meter.rs` | `MeasureResult`: LUFS I/S/M, `lra`, `true_peak_db`, `sample_peak_db`, `crest_db`, `clipped_samples`, `correlation` (one scalar), `mono_penalty_db`, `bands` {low/mid/high/air} |
| Metering (unused on wire) | `resonance-metering` | `spectrum/offline.rs` (Welch, 8192-pt), `spectrum/octave.rs` (1/6-oct), `plr.rs` (PLR/PSR), `correlation.rs` (~100 ms sliding) |
| Reference A/B | `resonance-audio/src/engine/reference.rs` | Loudness-matched reference player with markers. **GUI-only** |
| Normalized export | `resonance-audio/src/engine/bounce/normalize.rs` | `NormalizeSpec` (target LUFS, ceiling dBTP) exists, but `render.mixdown` doesn't expose it |
| Automation | `automation-control-api.md` | Planned. Needed for delay throws and send rides. Its slice A0 fixes `meter.*` ignoring automation, which is a prerequisite for measuring any automated depth move |
| Skills | `resonance-agent-plugin/skills/{mixing,mastering}` | Mixing: faults → balance → tone → dynamics/width, one class per pass, no warmth/depth/width guidance. Mastering: stage-by-stage via `*_on` keys, "sterile" has no measurement, and it never names `sat_character`/`sat_shaper`/EQ band keys/`img_*` |

Framework constraint: **a plugin cannot change its latency without a
restart** (gap F2, plugin-audit.md). Since ba todo #1296 (`8c341bf0`) a
plugin *can* report a new latency through
`HostHandle::set_latency_samples`, and `resonance-ir` does so for its
latency mode. But the change forces a deactivate → reactivate restart,
which is a dropout that flushes whatever audio is in flight (~0.4 s on
the mastering chain). That is no good for anything a user toggles while
listening. So the rule stands in practice. Oversampling either uses the
latency-free IIR `Oversampler`, or its factor is fixed at activation. A
linear-phase or lookahead option keeps a constant latency. The de-harsh
stage (W12) charges its one-frame latency whether it is on or off.

## 6. Proposed DSP additions

Primitives go in `resonance-dsp` (lowest layer they fit, per
ARCHITECTURE.md). Each new plugin needs one registration spot
(`plugins/<name>/` + workspace member).

### 6.1 `resonance-color` — a character plugin for tracks and busses

This is the single biggest gap. The mastering saturator is the only warmth
device, and warmth belongs on busses first (§4 step 3).

- **Modes** (each is a voicing of shared primitives, named generically):

  | Mode | What it is |
  |---|---|
  | `Tube` | Biased asymmetric soft curve, H2-dominant. Generalise the wavetable `Tube` shaper |
  | `Tape` | Soft shaper + speed-dependent head bump (peak, Q≈1.2, dip an octave up) + level-dependent HF loss (self-erasure: a shelf driven by an envelope) + optional flutter (0 = bypassed, no interpolation, as in ToTape6) |
  | `Transformer` | LF-weighted drive (pre-emphasis lowpass into the shaper, de-emphasis after) + sub-sonic HPF + small HF resonance |
  | `Console` | Airwindows-style `sin`-family per-channel curve at very low drive. Transparent when drive = 0 |
  | `Warm` | One-polarity saturation (the PurestWarm idea), even-only |

- **Controls:** `drive`, `bias` (asymmetry), `response` (tilt of *drive*,
  lows↔highs, the IVGI idea), `tone` (output tilt), `mix`, `auto_gain`
  (on by default: RMS-matched output, which is the loudness-confound fix),
  `output`, and `speed` / `flutter` in Tape mode only.
- **Anti-aliasing:** ADAA for the memoryless modes. Latency-free 2×/4× IIR
  oversampling as a param, since the half-band `Oversampler` reports no
  fixed latency. This is the only oversampler in the spec (decision D3).
- **Tape quality switch** (decision D2): `tape_quality` Standard / HQ.
  Standard is the shaper + filters model above: ADAA, cheap, default.
  HQ swaps the shaper for a Jiles-Atherton hysteresis stage (the Chow Tape
  approach: drive/sat/bias mapped to M_s, a, c; fixed k). It runs at
  forced 2× or 4× through the IIR oversampler, since ADAA doesn't apply
  to a stateful model. Both qualities share the head bump, HF loss and
  flutter filters, so switching changes only the nonlinearity. HQ ships
  in its own slice (W6b) with a solver choice and its own tests.
- **Presets** that skills can name: "Bus — Warm Glue", "Bass — Iron",
  "Vocal — Tube Air", "Drums — Tape 15", "Master — Subtle Tape".
- **Viz:** transfer curve plus a live harmonic bar display (H1–H7) from
  the probe math in §7.3.

Mastering's `sat_*` stage then gains the same mode set by sharing the
primitives. That removes the fixed tape voicing noted in plugin-audit.md.
The existing `sat_character` Tube↔Tape blend stays as-is, so saved
projects are unchanged.

### 6.2 `resonance-stereo` — a mono-safe width tool

| Control | Purpose |
|---|---|
| `width` | M/S side gain, 0–200% |
| `mono_below` + slope | Mono-maker / elliptical EQ |
| `widen_mode` | Off / **Decorrelate** (velvet-noise, pure side, default) / Diffuse (all-pass cascade) / Micro-shift (±cents + delays, with focus band) / Haas (with level offset and low exclude, flagged as a mono risk) |
| `widen_amount`, `focus_low/high` | Band restriction, so bass fundamentals stay dry |
| `balance`, `rotation` | Balance and stereo rotation |
| `solo_side`, `mono_check` | Audition toggles |
| Viz | Goniometer + correlation strip |

Invariant to test: **Decorrelate mode never changes the mono sum**
(bit-exact up to float rounding).

### 6.3 Mastering plugin extensions

- **M/S on the EQ stages.** A per-band `ms` selector (Stereo/Mid/Side) on
  the corrective and tonal stages. This covers the side air shelf, side
  low cut and side low-mid cut.
- **Per-band width.** The imager reuses the multiband crossovers, adding
  `img_b{n}_width`.
- **Clipper stage** before the limiter: `clip_on`, `clip_drive`, `clip_shape`
  (hard↔soft), oversampled 4–8× via IIR (no latency change).
- **De-harsh stage** (resonance suppressor, band-restricted to 1–8 kHz by
  default): `dh_depth`, `dh_sharpness`, `dh_selectivity`, attack, release,
  M/S, delta monitoring. Decision D4: this is a **full resonance
  suppressor** (soothe-style: many narrow, time-varying cuts on
  detected peaks relative to a smoothed spectrum), not a dynamic-EQ
  stand-in. It is a new DSP family, so it needs its own design note
  before W12 starts: detector (STFT vs filterbank), smoothing, latency
  (must be constant, per F2), and how `selectivity` is defined. The
  suppressor core lives in `resonance-dsp` so the `eq` plugin or a
  track-level insert can reuse it later.
- **Density/inflator option** on the saturator (odd polynomial with a
  curve control) for loudness without more limiting.
- Fix the stale module docs (`lib.rs` "Phase 2…", `chain.rs` "later
  phases…").

### 6.4 `eq`, `compressor`, `reverb`, `delay` extensions

| Plugin | Addition | Why |
|---|---|---|
| eq | Per-band M/S mode | M/S EQ on busses |
| eq | Band types **Tilt** (pivot freq, ±dB), **LF Lift+Dip** (Pultec-style), **Air** (shelf with a corner up to 40 kHz) | The three warmth/air moves as one knob each |
| eq | `auto_gain` | Loudness-matched EQ judgement |
| eq | Dynamic bands (threshold/ratio/attack/release per band, optional sidechain) | De-harsh and unmask on tracks |
| compressor | `release_mode` Auto (program-dependent, dual time constant) | Bus-glue norm |
| reverb | `wet_hpf`, `wet_lpf` (12/18 dB/oct, **before** the tank = Abbey Road) | Return EQ without a second plugin |
| reverb | Sidechain key + `duck_amount/attack/release` | Ducked vocal reverb in one plugin. The key port already exists in the compressor/gate pattern |
| reverb | `er_tail_balance` (depth crossfade, ValhallaRoom-style) | Per-layer ER vs shared tail |
| delay | A "Haas — Safe" preset (short time, no feedback, low cut, R tap −4 dB); no new DSP | Width lives in `resonance-stereo`; keep delay scope down |

### 6.5 Depth macro — defer

A `distance` macro (gain + HF shelf + send + pre-delay together, like TDR
Proximity) is tempting. However, it crosses the track/send boundary and
duplicates what a skill can do with existing controls. Build the skill
first (§8); revisit only if agents consistently get the depth recipe
wrong.

## 7. Proposed analysis and control-API additions

Resonance-metering holds the pure math, `engine/bounce/measure.rs` the
engine side, `methods/meter.rs` the wire and `tools/meter.rs` the MCP
tool. Update `reading-meters.md` in the same slice as each field.

### 7.1 Richer `meter.measure` / `meter.stems` (additive fields)

Everything is opt-in through `detail: ["spectrum", "stereo", "dynamics"]`,
so the default payload stays small for token cost.

- **`spectrum`:**
  - `third_octave` (31 bands, dB, 20 Hz–20 kHz)
  - `tilt_db_per_oct` (regression over 100 Hz–10 kHz)
  - `centroid_hz`
  - `lowmid_presence_db`
  - `presence_peakiness_db`
  - `peaks` (top 5 narrow 1/6-oct resonances relative to the smoothed LTAS: freq, excess dB)
- **`stereo`:**
  - `bands[8]` of `{lo_hz, hi_hz, correlation, side_mid_db, mono_loss_db}`
  - `correlation_windows` summary (`pct_below_0_3`, `worst`, `worst_at_seconds`)
  - `balance_db`
  - `one_sided: bool`
  - `haas_lag_ms` (lag of the strongest L/R cross-correlation peak in 1–35 ms, if its normalized value > 0.5)
- **`dynamics`:** `plr_db`, `psr_db` (from `plr.rs`).

### 7.2 `meter.compare` — loudness-matched A/B

This is the single most important tool for warmth.

`meter.compare {target, range, a: "current"|snapshot_id, b: ..., match: "lufs"}`
renders both states, gain-matches B to A's integrated LUFS, and returns
the *deltas* of every proxy in §2.

The flow needs a `meter.snapshot` (store the measurement plus the gain
offset) so the agent can do: snapshot → change → compare. Without it,
every "is it warmer?" judgement is confounded by level.

### 7.3 `plugin.probe` — harmonic signature of an insert chain

Runs a 1 kHz sine (plus an optional SMPTE 60 Hz + 7 kHz IMD pair) at a
given dBFS through a track, bus or master insert chain *offline*, and
returns:

- `thd_pct`
- `h[2..9]` in dBc
- `h2_h3_db`
- `decay_db_per_order`
- `aliasing_floor_dbc`
- `imd_pct`

This lets an agent set drive to a THD target (§2.1) instead of guessing
from a knob position. It reuses the bounce path with a synthetic source.

### 7.4 Expose the mastering assistant

`master.assist {mode: "genre", genre} | {mode: "reference", pool_asset_id}`
→ returns `decide.rs`'s suggestions with rationale **without applying
them**. The agent applies the parts it agrees with through the normal
param tools. Persist genre and reference in plugin state (fixes the
plugin-audit finding).

Genre targets become *bands* (min/max per 1/3-oct), which is the
Tonal-Balance-Control model, rather than single curves. Decision D6: the
bands are **built in, from published averages**. Each genre gets a
Pestana-style slope (about −4.5 to −5 dB/oct over 100 Hz–4 kHz) with
genre-specific low-end and top offsets and a ± tolerance per band. They
replace the heuristic curves in `assistant/targets.rs`. Reference tracks
(§7.5) stay a separate comparison mode and don't generate targets.

### 7.5 Reference tracks

`reference.load {pool_asset_id}` and `meter.measure {target: {reference}}`
let an agent compare tilt, width per band and PLR against a commercial
reference that the user supplies. Reuse `engine/reference.rs` and the
assistant's loader.

### 7.6 Depth measurement

`meter.stems {detail: ["depth"]}` returns `drr_db`, `hf_tilt_db`, and a
`layer_hint` (front/middle/back from DRR tertiles) per track.

Decision D5: DRR is an **estimate from the existing single stems pass**,
not per-source renders:

`drr_db_estimate ≈ −(send_level_db + return_fader_db + return_wet_gain_db)`

Here `return_wet_gain_db` is the return bus's output energy relative to
its summed input, measured once per return in that same pass. A
pre-fader send also adds the source's own fader gain. The estimate ignores the source's spectrum through
the reverb, which is good enough for the *ordering* check it exists for
(§2.3). The field is labelled `drr_db_estimate` so nobody reads it as a
measurement.

### 7.7 Delivery

Expose `NormalizeSpec` on `render.mixdown`: `normalize: {target_lufs,
ceiling_dbtp}`. Add a `platform` shorthand (spotify/apple/youtube/club)
that maps to the §3.4 targets.

## 8. What goes in skills vs what goes in Resonance

**The line:** a skill holds *judgement*: which problem, in which order,
how much, and when to stop. Resonance holds *anything computed or
measured*, and anything that must stay true when the code changes. A
number belongs in a skill only if it is a craft target (−14 LUFS, bus THD
0.5–3%) and not a property of our DSP.

### 8.1 Store in skills

- **The vocabulary table (§1).** Adjective → signal meaning → lever. This
  is what turns "warmer, more depth" into a plan.
- **Procedures (§4)** for warmth, width and depth. Fixed order, one class
  of move per pass, measure at matched loudness, stop rules (crest falling
  faster than loudness rises; tilt past target; PSR < 8).
- **Targets and thresholds (§2, §3.4)** as tables in `references/`.
- **The per-role staging table:** pan, width, send, pre-delay and layer
  per role (kick, bass, snare, lead vocal, backing, guitars, keys, pads,
  lead synth, FX). The skill must branch on *roles it infers from
  `song_tracks`*, never on track names or genres. The README rule forbids
  a skill that names a track or genre, so a genre goes in as a
  *parameter* the skill reads from the user, not as a branch baked in.
- **Resonance-specific gotchas:**
  - The pan law is stereo balance (centre = unity), so compensate the
    fader about 3 dB after hard panning.
  - `bands` is relative, so never compare it across tracks without
    matching.
  - Busses overlap their members.
  - `meter.*` ignores automation until A0 lands.
  - A freshly inserted plugin does nothing until configured.
- **Plugin param-key maps** (`references/plugin-keys.md`): which string
  keys implement each lever (`sat_character`, `img_side_hpf_freq`,
  `predelay`, …). Extend `agent_plugin_lockstep.rs` to check that every
  key named there exists in the plugin's param table, in the same way it
  already checks tool names. This is the cheapest guard against skill rot.
- **Preset names** to start from. Also lockstep-checked.

### 8.2 Don't store in skills

- Harmonic/THD maths, spectrum maths, the DRR computation: put them in the
  meters (§7).
- Mode voicings, curve formulas, oversampling choices: put them in plugin
  code and presets.
- Anything project-specific (this song's vocal chain, the user's taste for
  dark mixes). That belongs in the project's own `CLAUDE.md` /
  `.claude/skills`, per the plugin README.

### 8.3 Skill layout

The skills are per-craft. Warmth is a mixing *and* mastering concern, so
it doesn't need a skill of its own. Spatial staging is a distinct craft
with its own procedure, so it gets one.

| Skill | Change |
|---|---|
| `mixing` | Add a **tone/character** pass (bus warmth, de-harsh, tilt) after Tone, and a pointer to `spatial`. Add `references/character.md` (vocabulary, warmth procedure, THD targets) and `references/roles.md` (per-role table) |
| **`spatial`** (new) | Width + depth: layer assignment, mono-maker, pan + fader compensation, decorrelation, shared room send, return EQ/ducking, pre-delay from tempo, DRR ordering check. References: `references/width.md`, `references/depth.md` |
| `mastering` | Name the actual keys (`sat_*`, EQ band keys, `img_*`, new `clip_*`); add a "sterile" test from the tilt and harmonic probe; add a reference-track branch through `master.assist`; `references/delivery.md` (platform targets, PLR/PSR, dither rule) |
| `mixing/references/reading-meters.md` | Document every new field in §7, including `one_sided` and the S/M ↔ correlation relation |

Bump `plugin.json` `version` with every skill change. The user-level
`resonance-studio` skill should point at these rather than duplicate them.

## 9. Build order (vertical slices)

| # | Slice | Contents | Exit criterion |
|---|---|---|---|
| W0 | Meter spectrum | `detail: spectrum` (1/3-oct, tilt, centroid, LM/presence, peakiness, peaks) + reading-meters | Pink noise measures tilt −3.0 ± 0.1 dB/oct; a +3 dB shelf moves the right bands |
| W1 | Meter stereo + dynamics | Per-band correlation, S/M, mono loss, windows, balance, `one_sided`, Haas lag; PLR/PSR | Synthetic cases: mono (r = 1), hard-pan (`one_sided`), 10 ms Haas (lag 10 ± 0.1 ms), antiphase |
| W2 | `meter.snapshot` + `meter.compare` | Loudness-matched deltas | Same state compares to all-zero deltas; +3 dB gain alone compares to ≈0 after matching |
| W3 | `plugin.probe` | THD/H2/H3/aliasing/IMD | tanh at known drive matches analytic harmonics; the mastering saturator at Tape shows H2 > 0 |
| W4 | Skills v1 | `character.md`, `roles.md`, `spatial` skill, mastering key names, lockstep param-key check | Lockstep passes; a scripted agent session on a test project produces a measurable tilt/THD change with matched LUFS |
| W5 | `resonance-dsp` primitives | Asymmetric/one-polarity shapers with ADAA, head-bump/HF-loss filters, LF-weighted drive, velvet-noise decorrelator, all-pass cascade, M/S helper, soft/hard clipper | Unit tests on harmonics, mono invariance, aliasing floor |
| W6 | `resonance-color` | §6.1 plugin + presets + viz, Standard tape quality | Per-mode harmonic signatures pinned; auto-gain keeps ±0.5 LU; goldens with non-silent scenarios |
| W6b | Tape HQ | Jiles-Atherton hysteresis behind `tape_quality`, forced IIR oversampling | Hysteresis loop shape matches the reference model; no NaN/instability at max drive on full-scale noise; HQ vs Standard harmonic delta pinned |
| W7 | `resonance-stereo` | §6.2 | Decorrelate mode leaves the mono sum unchanged; the mono-maker's low-band correlation reads ≥ 0.99 |
| W8 | Reverb + EQ extensions | Wet HPF/LPF, ducking, ER/tail balance; EQ M/S, Tilt/LF-Lift/Air, auto-gain | Existing presets and goldens unchanged (defaults bit-transparent) |
| W9 | Mastering extensions | EQ M/S, per-band width, clipper, sat modes, inflator | Projects saved before the change render bit-identical |
| W10 | `master.assist` + reference | §7.4–7.5; persist genre/reference | The assistant's suggestion round-trips over MCP; the reference measures like a pool clip |
| W11 | Depth metering + delivery | §7.6, §7.7 | DRR ordering on a 3-layer fixture; normalize hits −14 ± 0.2 LUFS, ≤ −1 dBTP |
| W12 | De-harsh resonance suppressor | §6.3, core in `resonance-dsp` | Design note approved first; a synthetic 3.2 kHz resonance is cut ≥ 6 dB while a broadband signal changes < 0.5 dB; constant reported latency |

W0–W4 alone make the *existing* DSP usable for warmth: the mastering
saturator, EQ shelves, the reverb and `img_*`. Ship them first and
re-assess the DSP list with real agent sessions.

Dependencies:

- W11 depends on automation A0 (meters honouring automation) for
  throw-heavy depth.
- W6 and W7 are independent of each other and can run in parallel
  worktrees. The memory note applies: force `git merge master` plus an
  ancestry check in each agent prompt.

## 10. Decisions (2026-09-28)

| # | Question | Decision |
|---|---|---|
| D1 | Separate plugin or grow `eq`/mastering? | **Separate `resonance-color`.** Mastering's `sat_*` shares the `resonance-dsp` primitives, not code paths |
| D2 | Hysteresis tape model in v1? | **Both:** shaper + filters as Standard (default), Jiles-Atherton hysteresis as an HQ option (`tape_quality`, slice W6b) |
| D3 | Oversampling latency | **IIR only, latency-free.** The existing half-band `Oversampler` everywhere; no linear-phase option, so F2 is not a prerequisite |
| D4 | De-harsh | **Full resonance suppressor** (W12), with a design note first. The `eq` dynamic bands in §6.4 stay as a separate, simpler feature |
| D5 | Per-source DRR | **Approximate** from the single stems pass (`drr_db_estimate`, §7.6); no per-source renders |
| D6 | Genre target bands | **Built-in published averages** (Pestana-style slope ± tolerance per genre, §7.4); reference tracks compare but don't generate targets |

Still open: none blocking. Revisit D5 if agents misorder layers on real
projects.

## 11. Tests (per CLAUDE.md conventions)

- New plugin tests go in the plugin crate's own `tests/` (no inline
  tests). App-level tests are modules in the existing group binaries
  (`mixer`, `control`), not new top-level files.
- **Harmonic assertions** replace "sounds warm":
  - pinned H2/H3 per mode
  - decay per order
  - aliasing floor ≤ −90 dBc at 4× / ADAA for a 5 kHz tone at +12 dB drive
- **Mono-sum invariants** for widening (Decorrelate, M/S at width 1.0).
- **Goldens:** every scenario pins its params and asserts non-silence
  per scenario (silent goldens are vacuous). Run once with
  `RESONANCE_RENDER_THREADS=8` after touching the render path (W11).
- **Lockstep:** extend `agent_plugin_lockstep.rs` to param keys and
  preset names named in skills.
- **Back-compat:** all new params default to bit-transparent; a pre-change
  project renders bit-identical (W8, W9).

## 12. Sources

**Warmth, saturation, mastering**

- Chowdhury, *Real-time physical modelling for analog tape machines*, DAFx-19: https://ccrma.stanford.edu/~jatin/420/tape/TapeModel_DAFx.pdf
- AnalogTapeModel: https://github.com/jatinchowdhury18/AnalogTapeModel
- Chowdhury on ADAA: https://jatinchowdhury18.medium.com/practical-considerations-for-antiderivative-anti-aliasing-d5847167f510
- Airwindows source (Density, Console7): https://github.com/airwindows/airwindows
- Airwindows PurestWarm: https://www.airwindows.com/purestwarm/
- Airwindows ToTape6: https://www.airwindows.com/totape6/
- RCInflator2 (Oxford Inflator clone): https://github.com/ReaTeam/JSFX/blob/master/Distortion/RCInflator2_Oxford.jsfx
- Geddes & Lee, *Auditory perception of nonlinear distortion*: https://www.semanticscholar.org/paper/Auditory-Perception-of-Nonlinear-Distortion-Geddes-Lee/3cda726d86cb37a13d09c3f449abc50934072142
- Pestana, Reiss & Barbosa, *Spectral characteristics of popular commercial recordings 1950–2010*: https://www.researchgate.net/publication/274511175
- Endino tape response curves: https://www.endino.com/graphs/
- Kazrog True Iron manual: https://www.barryrudolph.com/recall/manuals/kazrog_true_iron.pdf
- Klanghelm IVGI: https://klanghelm.com/contents/products/IVGI
- FabFilter Saturn 2 band controls: https://www.fabfilter.com/help/saturn/using/bandcontrols
- Pultec trick: https://abbeyroadinstitute.nl/blog/demystifying-the-pultec/
- EQP1A WDF model: https://github.com/ABSounds/EQP1A-WDF
- Maag Air Band: https://maag.audio/maag-air-band/
- TDR SlickEQ: https://www.tokyodawn.net/tdr-vos-slickeq/
- TDR Kotelnikov manual: https://docs.tokyodawn.net/kotelnikov-manual/
- soothe2 manual: https://oeksound.com/manuals/soothe2/
- Ozone Imager: https://s3.amazonaws.com/izotopedownloads/docs/ozone9/en/imager/index.html
- Tonal Balance Control target curves: https://s3.amazonaws.com/izotopedownloads/docs/tonal-balance-control/meters-and-target-curves/index.html
- Low End Focus: https://www.izotope.com/community/blog/the-development-of-low-end-focus
- FabFilter Pro-L 2 advanced settings: https://www.fabfilter.com/help/pro-l/using/advancedsettings
- StandardCLIP manual: https://www.siraudiotools.com/StandardCLIP_manual.php
- Spotify loudness normalization: https://support.spotify.com/us/artists/article/loudness-normalization/
- PLR: https://productionadvice.co.uk/plr/

**Width and depth**

- SOS, phase-correlation meters: https://www.soundonsound.com/sound-advice/q-what-are-my-phase-correlation-meters-telling-me
- KERN, Haas and all-pass decorrelation: https://kernaudio.io/guides/stereo/haas-and-allpass-decorrelation
- Alary, Politis & Välimäki, *Velvet-noise decorrelator*, DAFx-17: http://www.dafx17.eca.ed.ac.uk/papers/DAFx17_paper_96.pdf
- Optimized velvet decorrelators: https://github.com/SebastianJiroSchlecht/OptimizedVelvetDecorrelators
- SOS, LCR panning: https://www.soundonsound.com/techniques/lcr-panning-pros-and-cons
- SOS, Haas mono compatibility: https://www.soundonsound.com/sound-advice/q-can-haas-delays-be-mono-compatible
- Soundtoys MicroShift manual: https://www.soundtoys.com/wp-content/uploads/MicroShift-Manual.pdf
- SOS, Brainworx bx_digital: https://www.soundonsound.com/reviews/brainworx-bx-digital
- Elliptical EQ: https://adrianmilea.com/how-to-use-an-elliptical-eq/
- Polyverse Wider manual: https://polyversemusic.com/docs/wider-manual/
- Sonible, 9 rules of depth: https://www.sonible.com/blog/rules-of-depth/
- Joe Lambert, the depth dimension: https://joelambertmastering.com/mix-tips-from-your-mastering-engineer-5-use-the-depth-dimension/
- ValhallaRoom early controls: https://valhalladsp.com/2011/05/18/valhallaroom-the-early-controls/
- Abbey Road reverb trick: https://flypaper.soundfly.com/produce/the-abbey-road-trick-how-to-eq-reverb-sends-to-free-up-space-in-a-mix/
- Sidechained reverb: https://flypaper.soundfly.com/produce/hold-up-can-you-sidechain-reverb/
- Voxengo Correlometer: https://www.voxengo.com/product/correlometer/
- Trackspacer: https://www.wavesfactory.com/audio-plugins/trackspacer/
- Neutron Unmask: https://s3.amazonaws.com/izotopedownloads/docs/neutron4/en/unmask/index.html
- Air absorption (ISO 9613-1): https://dougjam.github.io/demos/atmospheric-absorption/
