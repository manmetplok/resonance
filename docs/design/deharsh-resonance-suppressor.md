# W12: the de-harsh resonance suppressor

Design for `warmth-width-depth.md` slice W12 (§3.1 soothe2 row, §6.3 de-harsh
bullet, §9 W12, §10 D3/D4). Written against master `3d295b5f`. Design only:
no DSP code lands with this note.

**Recommendation in one paragraph.** Build it as a **stereo STFT spectral
gain mask**. The frame is 2048 samples (at ≤ 48 kHz), the hop is 256
(87.5 % overlap), with a periodic Hann window for both analysis and
synthesis (WOLA). The detector computes a **detection spectrum** per bin:
power averaged over a triangular kernel whose width is the cut width,
`f/Q`, and integrated over ~10 ms. It compares that to a **reference**:
a peak-excluded, log-uniform, 1-octave moving average of the same detection
spectrum in dB. The **excess** `E = D − R` (dB) drives a feed-forward cut
with a soft knee and infinite ratio above `dh_selectivity`, capped at
`dh_depth`. The cut is smoothed per bin with attack and release at frame
rate, then applied as a real gain to the spectrum. **Latency is exactly one
frame, 2048 samples (42.7 ms at 48 kHz), in every parameter state,
including off.** When off, the stage outputs a bit-exact 2048-sample delay
line. The core is `resonance_dsp::deharsh::ResonanceSuppressor`. The stage
sits in the mastering chain directly after the corrective EQ and before
the glue compressor.

---

## 1. What the control set has to mean

soothe2's manual (§12 of the spec) defines the controls only by feel.
**Depth** means "more reduction". **Sharpness** means "deeper, narrower
cuts". **Selectivity** means "only prominent resonances". **Attack** is
"always relatively faster on high frequencies". **Soft/hard** means "less /
more level-dependent". It also has mix, delta, M/S with a link %, a
sidechain, and sensitivity-shaping bands. The manual says nothing about
latency or method. An agent cannot tune a knob that has no physical
meaning, so every `dh_` param below is defined in dB, Hz, Q or ms, with a
steady-state rule a test can check (§4).

## 2. Detector: STFT, not a filterbank

### 2.1 The two candidates

| | **STFT gain mask** (chosen) | **Filterbank + dynamic bells** |
|---|---|---|
| Shape | Per-bin analysis, per-bin real gain, WOLA resynthesis | 1/6–1/12-oct constant-Q or ERB bandpass detectors; each drives a minimum-phase dynamic bell in a series cascade |
| Latency | One frame, constant (2048 @ 48 k) | 0 |
| Frequency resolution | 23.4 Hz bins everywhere; a cut can land on *any* frequency | Fixed centres. 1–8 kHz is 18 bands at 1/6 oct, 36 at 1/12 oct. A resonance between two centres is cut by two overlapping bells at the wrong Q |
| "Smoothed spectrum" reference | Natural: the full spectrum is at hand every hop | Only 18–36 band levels. A 1-octave reference is 6–12 points, too coarse for peak exclusion |
| CPU, 48 kHz stereo | One complex FFT pair per hop for both channels (§2.4): ≈ 0.7 % of one core | 36 detector BPFs + 36 bells × 2 ch, each recomputing coefficients per block: ≈ 144 biquads/sample, plus per-sample `sin`/`cos` for the coefficients. Similar or worse |
| Artefacts | Time-domain aliasing if the gain mask is too sharp in frequency; "musical noise" if per-bin gains fluctuate (§2.5) | Coefficient-modulation zipper on fast bells; phase distortion that piles up through a 36-deep cascade, which is audible on a master |
| Time resolution | Gain updates every 5.3 ms. The window smears a cut over ~21 ms (Hann equivalent duration), and that acts as lookahead | Per sample in principle. In practice a Q≈17 bell rings for ~1/BW ≈ 5 ms at 3.2 kHz, so it is barely faster |

### 2.2 Why STFT

The feature is defined as cuts on peaks *relative to a smoothed spectrum*
(D4). That is a spectral-envelope operation, and it needs the spectrum. A
filterbank can only approximate it at the resolution of its bands, and the
approximation fails exactly where the effect matters: narrow resonances
between band centres. Latency is the only real cost of the STFT, and the
mastering plugin already carries 18,672 samples of it (§5.2). Another
2,048 is +11 % on a plugin that no one tracks through. The hybrid option,
an STFT sidechain steering a handful of time-domain bells, still needs the
lookahead to be on time, so it has the same latency and adds bell-tracking
modulation. It is rejected.

### 2.3 Geometry

| Frame N | Latency | Bin spacing | At 1 kHz, 1/12 oct (58 Hz) is | Verdict |
|---|---|---|---|---|
| 1024 | 21.3 ms | 46.9 Hz | ~1 bin: cannot separate | Too coarse for the default 1 kHz band edge |
| **2048** | **42.7 ms** | **23.4 Hz** | ~2.5 bins | Chosen |
| 4096 | 85.3 ms | 11.7 Hz | ~5 bins | Smears snare and vocal consonants over 40+ ms |

- **Window:** periodic Hann for both analysis and synthesis. Hann² overlap-adds
  to the constant `Σ w² = (3/8)·N/H = 3` at hop N/8, so the output is
  scaled by 1/3. The
  implementation computes the constant numerically at construction and
  asserts it is flat to 1e-6 (COLA check, test T9). The synthesis window
  tapers the frame edges, which is where circular-convolution wrap from a
  gain mask lands.
- **Hop:** H = N/8 = 256. This is soothe's "resolution" knob pinned high.
  An 87.5 % overlap halves the per-bin gain step between frames compared
  with N/4, which is the main anti-zipper measure. It doubles the CPU,
  which is still negligible (§2.4).
- **Sample rate:** the same rule as `FirGeometry::for_sample_rate`. The
  base is kept up to 48 kHz and doubled per octave of rate above it
  (4096 at 96 k, 8192 at 192 k, capped at 8×). So resolution in Hz and
  latency in ms stay the same. At 44.1 kHz, N = 2048 (46.4 ms, 21.5 Hz).
  The geometry is fixed at construction (= activation).

### 2.4 How the cut is applied, and the cost

The cut is applied by spectral gain masking with weighted overlap-add. Per
hop:

1. Take the last N input samples of both channels. Window them, and pack
   them as `z = w·(a + j·b)`, where (a, b) is (L, R), or (M, S) in the M/S
   modes. Run one complex N-point forward FFT (rustfft, already a
   dependency; plan and scratch allocated at construction).
2. Unpack `A[k] = (Z[k] + Z*[N−k])/2` and `B[k] = (Z[k] − Z*[N−k])/(2j)`
   for bins 0..=N/2.
3. Run the detector (§3) on the bins inside the analysis span only. That
   is the band ±½ octave: 707 Hz–11.3 kHz at the defaults, ≈ 450 bins.
4. Multiply `Z[k]` by the real gains, `g_a[k]·A[k] + j·g_b[k]·B[k]`, which
   is re-packed Hermitian-symmetric. Bins outside the band get exactly 1.0
   and are skipped.
5. Run one inverse FFT. Its real and imaginary parts are the two
   channels. Apply the synthesis window, overlap-add into a 2N
   accumulator, and emit the H finished samples.

Cost at 48 kHz stereo is 187.5 hops/s × (two 2048-pt complex FFTs, about
10–15 µs with AVX, plus ~450 bins × ~40 flops of detector work). That is
**≈ 5–7 ms of CPU per second of audio, ≈ 0.6 % of one core.** The budget
test (T11) allows 2 %. At a 128-frame quantum, one callback runs at most
one iteration (every other callback), so there are no spikes.

### 2.5 Artefacts and how the design bounds them

- **Time-domain aliasing.** Multiplying by `g[k]` is circular convolution
  with the gain's impulse response. A gain curve that is smooth over ≥ 3
  bins has a response shorter than N/3. The Hann synthesis taper absorbs
  most of the wrap. The cut can never be narrower than the detection
  kernel (§3.1), whose floor is 3 bins. If T8 shows aliasing anyway, the
  fallback is 2× zero-padding (FFT 4096, window 2048). **Latency stays
  2048**, because latency is set by the window, not the FFT. CPU doubles.
- **Musical noise** (Cappé 1994). Per-bin gains that flicker with the
  periodogram's chi-squared variance make warbling tones. There are three
  defences. (a) The detector averages over the kernel and ~10 ms, so
  ν ≳ 10–30 degrees of freedom (§3.3). (b) The knee means small random
  excesses produce no cut at all. (c) Attack and release smooth each
  bin's cut in dB.
- **Pre-echo of cuts.** A cut computed from a frame is spread over the
  whole frame, so it can start ~20 ms before a resonance's onset. For a
  *cut* this is benign: it is inaudible pre-ducking, not a pre-echo of
  energy, and it doubles as lookahead.

## 3. Smoothing: detection spectrum, reference, excess

All three work on the per-hop power `P[k]`. In the linked Stereo mode this
is `P = (|A|² + |B|²)/2`, so both channels get the same gains and the image
holds. In the Mid+Side mode, M and S are each measured and cut
independently.

### 3.1 Detection spectrum D (the cut-width average)

1. **Frequency.** Take a triangular kernel over bins, with full width at
   half maximum `BW(f_k) = max(f_k / Q, 3·Δf)`, where `Q = dh_sharpness`
   and `Δf` is the bin spacing. It is implemented as two cascaded boxcars
   over a running prefix sum, so the work per bin is O(1) whatever the
   width. The average is taken in **power**, so D is "energy in a band as
   wide as the cut".
2. **Time.** A one-pole across hops with a fixed τ_det = 10 ms. This is
   not a user param. It exists only to add degrees of freedom.
3. `D_dB[k] = 10·log10(D[k] + ε)`. Use a fast `log2` approximation
   (~1e-4 dB error), since this runs ~450× per hop per channel.

### 3.2 Reference R (the "smoothed spectrum")

The reference is a **peak-excluded, log-frequency moving average of
`D_dB`** over a fixed 1-octave window (`[f/√2, f·√2]`). Each bin is
weighted by `1/f_k`, so the average is uniform in log frequency. That
makes it exact on any constant dB/oct tilt, such as pink noise or a
mastering-style −4.5 dB/oct slope. It takes two passes, both O(bins) with
prefix sums:

```
R1[k] = logmean_{1 oct}(D_dB)                 // plain average
R[k]  = logmean_{1 oct}(min(D_dB, R1))        // peaks clipped to R1, re-averaged
```

Why this form:

- **Log domain.** An average in dB follows spectral tilt and the broad
  tonal balance, so broad EQ shape is never mistaken for a resonance. It
  is the fractional-octave smoothing of Hatziantoniou & Mourjopoulos
  (JAES 2000), applied to a short-time spectrum.
- **Peak exclusion.** A plain average is pulled up by the peak it is
  meant to expose: a +12 dB, 1/12-oct peak raises a 1-octave mean by ~1 dB.
  Clipping to R1 and re-averaging removes most of that bias. It is one
  extra prefix-sum pass. A median would be exact but costs O(width) per
  bin with a sort.
- **Computed from D, not raw bins.** The mean of a raw periodogram in dB
  is biased 2.5 dB below its power mean (Euler–Mascheroni: E[ln χ²₂/2] =
  −γ). Comparing a power-averaged D with a dB-averaged raw spectrum would
  make broadband noise read +2.5 dB "resonant" everywhere. Taking both
  from D cancels that bias to within ~0.2 dB.
- **Not cepstral.** Low-quefrency liftering is a fixed-width smoothing in
  linear Hz, so a 1-octave window at 1 kHz would be 1/8 octave at 8 kHz.
  It costs a third FFT, and it cannot exclude peaks. The log-f moving
  average is the variable-width generalisation of what a lifter does.
- **Width fixed at 1 octave.** This is ≥ 2× the widest cut (Q = 3 ≈ 0.48
  oct), so a cut can never flatten its own reference. It is not a user
  param: two widths (cut and reference) under one "sharpness" would be
  unexplainable to an agent.
- The reference window extends past the band edges, which is why the
  analysis span is the band ±½ octave. Near Nyquist or DC it is truncated
  (one-sided), and that is fine at the default band.

### 3.3 Excess, and why broadband noise does not trigger

`E[k] = D_dB[k] − R[k]` is the peak height in dB above the local
smoothed spectrum. It is **level-independent**: scaling the input by any
gain leaves E unchanged. This is soothe's "soft mode" as a hard
guarantee. Hard mode is not offered.

Below an absolute floor, `R[k] < −100 dB` relative to a full-scale sine's
bin power, E is forced to 0, so silence, dither and reverb tails are never
"corrected".

On noise, D has ν ≈ 2·(kernel bins / 1.5) × (time dof) degrees of freedom.
This estimate was written for the draft default Q = 8: at 1 kHz
ν ≈ 10 (σ ≈ 1.9 dB), and at 3.2 kHz ν ≈ 30 (σ ≈ 1.1 dB). With T = 6 dB
and a 4 dB knee, a cut starts at E = 4 dB. That is a power ratio of 2.5,
which noise exceeds with probability ≈ 2·10⁻³ at 1 kHz and far less
higher up. Even those events land in the knee (< 0.5 dB cut) and are
smoothed by attack. At the shipped defaults (Q 24, T 5, set after phase
1) the kernel is narrower, so ν is about a third of that and the cuts
start at E = 3 dB. Noise then triggers small knee cuts more often, but
T2 measured every 1/3-oct band within 0.19 dB, well inside the 0.5 dB
bound. The measurement is the proof; this paragraph is only the
reasoning.

## 4. Params and their precise meaning

Mastering stage params use the `dh_` prefix. They go after the corrective
EQ's params in the flat list, in signal order, like the other stages
(`params/mod.rs`). Param ids are `stable_hash`ed strings, so inserting
them mid-list moves nothing. Everything is read once per hop. Nothing
needs a per-sample smoother, because every param acts through the
attack/release-smoothed cut.

| Key | Range, default | Precise meaning |
|---|---|---|
| `dh_on` | bool, **off** | Off: output = input delayed by exactly L samples, bit-exact. On/off crossfades over 10 ms (the chain's `BYPASS_XFADE_SECONDS` pattern). The STFT keeps running while off, so it is always warm |
| `dh_depth` | 0–24 dB, 6 | The cap on any single bin's cut. The steady-state rule is below |
| `dh_selectivity` | 0–18 dB, 5 | T: how far above the reference, in dB, a peak must stand (by E, §3.3) before it is cut. Higher = only prominent peaks |
| `dh_sharpness` | Q 3–24, 24 | The Q of each cut. The detection kernel's FWHM, and so the cut's width, is `f/Q` Hz, floored at 3 bins (70 Hz). The floor caps the effective Q at ~14 at 1 kHz and ~46 at 3.2 kHz |
| `dh_attack` | 5–200 ms, 10 | Time constant (to 63 %) for a bin's cut to *deepen*. Frame-rate one-pole, `a = exp(−H/(τ·fs))`. Below one hop (5.3 ms), cuts follow frame to frame |
| `dh_release` | 20–1000 ms, 100 | The same, for a cut to *recover* |
| `dh_low`, `dh_high` | 200 Hz–20 kHz; **1000 / 8000** | The band in which cuts may happen. The cut weight is 1 inside and falls on a raised cosine to 0 over 1/6 octave outside each edge. If `low > high`, the two are swapped. Detection and reference still see ±½ oct beyond the band |
| `dh_mode` | Stereo / Mid / Side / Mid+Side, Stereo | Stereo: L/R, linked detection, one gain curve, image preserved. Mid, Side: encode M/S (`resonance_dsp::ms_encode`) and process that channel only; the other gets unity gains. Mid+Side: M and S each detected and cut independently |
| `dh_mix` | 0–100 %, 100 | Linear, latency-aligned: `out = x_d + mix·(y − x_d)`, where `x_d` is the input delayed by L and y is the processed signal. At 0 it is exactly `x_d`. `dh_depth` is the better "less" control; mix is there for parallel-style blends |
| `dh_delta` | bool, off | Output `x_d − out`: what the stage removes, at the current mix. **`out(delta off) + out(delta on) = x_d`** by construction (T4) |

**Gain law and the steady-state rule.** For each bin with band weight
`w[k]`:

```
x      = E − T                     (T = dh_selectivity)
over   = 0                         if x ≤ −K/2
       = (x + K/2)² / (2K)         if |x| < K/2      (K = 4 dB fixed knee)
       = x                         if x ≥  K/2
target = w[k] · min(dh_depth, over)            // dB of cut, ≥ 0
cut    = attack/release one-pole toward target
g[k]   = 10^(−cut/20)
```

Outside the knee, **a steady peak that stands E dB above its reference
leaves the stage at `max(T, E − depth)` dB above it.** The stage pulls
peaks down to the selectivity line (infinite ratio), and never pulls any
of them by more than `depth`. This is the sentence the mixing and
mastering skills quote. The fixed ratio of ∞ is deliberate. A ratio knob
next to depth and selectivity would give three controls for two degrees
of freedom.

Deliberately left out of v1: soothe's frequency-dependent attack,
stereo-link %, external sidechain and sensitivity-shaping nodes, and a
soft/hard switch (§3.3 makes it always "soft"). Each can be added
additively later (§7).

## 5. Latency and where the code lives

### 5.1 Latency: exactly one frame, always

Streaming WOLA works as follows. The frame processed after input sample
`t−1` covers `[t−N, t)`. No later frame touches samples `[t−N, t−N+H)`, so
those are finished and are emitted over the next H sample periods, paired
with inputs t…t+H−1. **The latency is exactly N samples.**

| Sample rate | L (samples) | L (ms) |
|---|---|---|
| 44.1 / 48 kHz | **2048** | 46.4 / 42.7 |
| 88.2 / 96 kHz | 4096 | 46.4 / 42.7 |
| 176.4 / 192 kHz | 8192 | 46.4 / 42.7 |

`ResonanceSuppressor::latency()` returns N, a function only of the sample
rate fixed at construction. No param changes it: off, depth 0, mix 0,
delta, and every mode all give the same N. The off path is a `DelayLine`
tapped at N, so it is aligned and bit-exact.

**F2.** The spec treats "a plugin cannot report a latency change" as open.
Since ba todo #1296 (`8c341bf0`) that is no longer strictly true.
`HostHandle::set_latency_samples` exists, and `resonance-ir` uses it for
its latency mode. But a change forces a deactivate→reactivate restart.
On the master that is a dropout, and it flushes ~0.4 s of in-flight audio.
That is unacceptable for toggling one stage during an A/B. So this design
**does not use it**: the latency is constant, and the stage is charged
even when off, exactly like the linear-phase EQs today (their delta FIR
keeps their latency when every band is off).

### 5.2 Consequences for the mastering plugin

- `Chain::latency()` becomes the old sum plus `deharsh.latency()`: at
  48 kHz, 18,673 → **20,721** samples (389 → 432 ms). The whole-plugin
  bypass delay (`max_latency`) includes the new term.
- **Where the off-state delay sits (amended in phase 2).** A plain
  in-place delay when off would *not* keep existing projects
  bit-identical. Delaying the glue, saturator, tonal EQ, multiband,
  limiter and dither input by 2048 samples moves the linear-phase FIRs'
  hop grid against the signal, which changes their rounding. It also
  moves the dither RNG draw each sample gets, and the block at which
  automation lands. So the stage has two timing modes
  (`stages/deharsh.rs`):
  - **Inline:** the stage outputs the suppressor, and off is its
    bit-exact delay tap.
  - **Tail:** the stage is a wire, and the latency is a delay line after
    dither.

  The mode is decided on the first block after a reset: inline if
  `dh_on`, tail otherwise. The first switch-on in tail mode hands over
  once. The downstream input crossfades onto the suppressor's delayed
  output. The tail keeps delaying until that splice has come out of the
  downstream stages (their latency, plus half a frame), and then
  crossfades to the direct path. Both paths carry the same timeline
  there, and the switch falls halfway between where the splice appears
  on each, so neither is heard. The stage then stays inline until the
  next reset. With depth 0, a handover through tonal EQ, glue and
  limiter differs from an inline-from-the-start render by −127 dB re
  peak (`stages_deharsh.rs`).
- **Bounces of existing projects stay bit-identical.** A project that
  never engages the stage renders as the pre-W12 chain delayed by
  exactly 2048 samples, bit for bit. The bounce trims
  `master_fx_latency` (`bounce/wav.rs`, `stem.rs`), so its bounce is
  bit-identical. Live playback of the master is 43 ms later, which is
  irrelevant on a plugin that already adds ~0.4 s.
- **`tests/dsp_golden.rs` will move.** It stores a raw output tail without
  latency compensation, so the tail shifts by 2048 samples. That is a
  known, provable move, not a regression. The same holds for
  `w9_golden.rs` and `legacy_state.rs`, which also store raw output.
  **Done in phase 2.** All 35 renders of the three files (every scenario,
  every legacy blob, and the W9 "stripped" renders) were dumped in full
  on the pre-W12 code and again after the change. Each post-change
  stream is exactly 2048 zero samples followed by the pre-change stream,
  bit for bit, on both channels. The goldens were re-blessed on that
  proof. `dsp_golden` needed `PRIME_BLOCKS` 56 → 60 for its 384-frame
  scenario, and gained two de-harsh scenarios (inline, and the handover
  with Mid+Side and mix).

### 5.3 Code layout

**`resonance-dsp/src/deharsh.rs`** (a `pub mod`; framework-agnostic, no
plugin trait, per ARCHITECTURE.md). It depends only on `rustfft` and
existing primitives (`DelayLine`, `ms_encode/decode`, `fill_hann_window`):

- `StftGeometry { frame, hop }` plus `for_sample_rate(sr)`, following the
  `FirGeometry` scaling rule.
- `SuppressorConfig { enabled, depth_db, selectivity_db, q, attack_ms,
  release_ms, low_hz, high_hz, mode, mix, delta }`, a plain struct.
- `ResonanceSuppressor::new(sample_rate)`, with `latency()`, `reset()`,
  `process_stereo(&mut [f32], &mut [f32], &SuppressorConfig)`,
  `process_mono(…)` for a future mono track insert, and read-outs
  `max_cut_db()` and `cut_curve(&mut [f32])` for meters and the editor.
  It takes arbitrary block sizes via FIFOs, like `FftConvolver`, and
  **allocates nothing after `new`**.
- `set_phase_offset(n)`, the DSP-16 stagger hook, so a host running
  several instances (or the chain's ten convolvers) can move when its FFT
  runs.

Reuse later: the `eq` plugin, or a track-level "De-harsh" insert, wraps
the same struct with its own param prefix. A track insert pays 43 ms of
PDC. If that is a problem, a low-latency geometry (N = 1024, 21 ms; worse
below ~2 kHz) can be a construction-time choice. It is per instance and
constant, so it is F2-safe.

**`plugins/resonance-mastering`**: `stages/deharsh.rs` (a thin wrapper,
like `stages/linear_phase_eq/convolver.rs` around `FftConvolver`),
`params/deharsh.rs` (`DeharshParams`, `PARAM_COUNT = 11`, `snapshot()`),
wiring in `params/mod.rs` (a `DH_BASE` after `CORRECTIVE_BASE`), and
`chain.rs`.

### 5.4 Position in the chain

```
input trim → corrective EQ → DE-HARSH → glue → saturator → tonal EQ
           → multiband → imager → limiter → dither
```

- **After the corrective EQ.** Static fixes first, time-varying fixes
  second. That is the "corrective" family in §3.4's standard order, and a
  future M/S corrective EQ (W9) sits with it. It also means the user's
  own corrective cuts are not fought by the suppressor.
- **Before the glue compressor and the multiband.** Otherwise both
  compressors pump on the resonant peaks the suppressor is about to
  remove. That is the main reason a suppressor goes early.
- **Before the saturator.** Saturating a resonance multiplies it into
  harmonics and IMD at 2f, 3f and f₁±f₂, which no later cut can separate
  cleanly.
- **Cost of the position:** harshness the saturator *adds* is not caught.
  The tonal EQ after it handles that statically. A post-saturator
  position option is an open question (§7), not a v1 feature.
- The module docs in `chain.rs` are stale ("Later phases will add…").
  Fix them in the same slice (§6.3 asks for this).

## 6. Test plan

Core tests go in `resonance-dsp/tests/deharsh.rs`. Stage and plugin tests
go in `plugins/resonance-mastering/tests/stages_deharsh.rs`. There are no
inline tests. Every signal is seeded and deterministic. Every test pins
all the params it depends on, and each scenario asserts the output is not
silent. Levels are measured with Welch PSDs (reuse
`resonance-metering::spectrum::offline`, which is a dev-dependency only)
after skipping 0.5 s of settling.

**Exit criteria (§9 W12):**

- **T1 — resonance is cut ≥ 6 dB.** Params: 10 s of seeded pink noise at
  −18 dBFS RMS through a +15 dB, Q 10 bell at 3.2 kHz, then the stage with
  depth 12, selectivity 5, Q 24, attack 10, release 100, band 1–8 kHz,
  Stereo. A second test, `defaults_meet_the_exit_criterion`, runs T1 on
  the enabled defaults with depth raised to 12, and T2 on the unchanged
  enabled defaults, so the defaults cannot drift from the exit criterion.
  At the default depth of 6 dB, the depth cap limits T1 to 5.8 dB. The level in the 1/12-oct band around 3.2 kHz drops by
  **≥ 6 dB** relative to the input. Measured in phase 1: 7.8 dB. An
  earlier draft pinned selectivity 6 at Q 8 and predicted ~8 dB, but
  that setting measures only 3.5 dB. The Q 8 kernel averages the Q 10
  resonance down to E ≈ 9 dB. Also assert that the 1/3-oct bands
  centred 1 oct away change by < 1 dB, so the cut is local and not a
  broad dip.
- **T2 — broadband is untouched.** Same params, pink noise without the
  bell. Every 1/3-oct band from 1 to 8 kHz, and the total, changes by
  **< 0.5 dB**. Repeat on white noise and on noise tilted −4.5 dB/oct,
  which checks the exact-on-tilt claim of §3.2.
- **T3 — latency is constant.** `latency()` equals N at 44.1/48/96/192 kHz
  and does not change across a sweep of every param, including on/off,
  delta, mix 0 and every mode. With on and depth 0, an impulse at n peaks
  at n + N. At plugin level, `latency_samples()` is identical before and
  after each `dh_*` change, with no restart requested, and equals the
  pre-W12 sum plus N.

**Further tests:**

- **T4 — delta reconstructs.** For mix ∈ {0.3, 1.0} on the T1 signal,
  `out(delta off) + out(delta on)` equals the input delayed by N within
  1e-6 peak.
- **T5 — off is bit-transparent.** With `dh_on = false`, the output is
  bitwise the input delayed by N. At chain level with every stage off,
  the output is bitwise the input delayed by `Chain::latency()`.
- **T6 — WOLA identity.** On, with depth 0 (unity gains): the output
  matches the delayed input within 1e-6 (−120 dB). This bounds the
  reconstruction error.
- **T7 — level invariance.** T1's signal at −6 dB and −30 dB: the cut at
  3.2 kHz differs by < 0.2 dB. Silence in gives exact zeros out. Floor
  test: a −110 dBFS resonance gets no cut.
- **T8 — no zipper or aliasing.** (a) A 3.2 kHz sine plus pink noise, with
  depth, selectivity, sharpness and `dh_low` automated every block
  (random walk, seeded). Energy more than ±100 Hz from 3.2 kHz, minus the
  same measure for static params, stays below −60 dBc. (b) A 500 Hz sine
  outside the band with every param automated: the output equals the
  delayed input within 1e-6, so out-of-band gains are exactly 1. (c)
  Toggle `dh_on` every 50 ms: the largest sample-to-sample step stays
  within the 10 ms crossfade bound.
- **T9 — COLA and packing.** The synthesis normalisation is flat to 1e-6.
  Stereo packing: a signal only in L leaves R exactly silent, in both L/R
  and M/S modes.
- **T10 — modes.** Stereo: a hard-left resonant signal plus a centred
  broadband signal keeps its L/R level ratio per band within 0.1 dB
  (linked). Mid: the side channel equals the delayed side within 1e-6.
  Mid+Side: a resonance only in S is cut, and M is untouched within 0.5 dB.
- **T11 — timing and CPU.** A resonance switched on at t0 reaches 63 % of
  its final cut within `attack + N/2` samples. Release is tested the same
  way. For CPU, a release-build timing test marked `#[ignore]` (run by
  hand, like the editor tests, because concurrent `run-tests.py` runs make
  wall-clock limits flaky) processes 60 s of 48 kHz stereo in < 2 % of
  real time, taking the best of 3 runs.
- **T12 — RT-safe.** No allocations inside `process_stereo` under param
  automation. It runs as its own test binary with a counting
  `#[global_allocator]`, the `linear_phase_rt_alloc.rs` pattern.
  Robustness: no NaN or Inf on full-scale noise, DC, alternating ±1, and
  denormal-level input.
- **T13 — golden.** Add a `deharsh_on` scenario to the mastering
  `dsp_golden.rs` with pinned params and a non-silence assertion. The
  existing scenarios shift and are re-blessed under the proof in §5.2.

## 7. Risks and open questions

1. **Musical noise on dense material** (cymbals, distorted guitars). The
   ν estimate in §3.3 is for stationary noise. Real material fluctuates
   faster. Mitigation, if listening or T8 flags it: raise τ_det, or smooth
   the cut across frequency with the same triangular kernel (this widens
   cuts by √2 and is invisible to the param meaning if Q is compensated).
2. **Aliasing with sharp cuts at high Q.** The 3-bin floor should hold it.
   The zero-padding fallback (§2.5) keeps latency unchanged. Decide only
   if T8 fails.
3. **Default calibration.** The defaults are depth 6, selectivity 5,
   Q 24 (decided after phase 1). The draft's Q 8 / T 6 cut the T1
   resonance by only 3.5 dB. The default Q sits at the top of its range,
   and a test pins the defaults to the exit criterion. They are measured,
   not heard: a listening pass should confirm them before the skills
   quote them.
4. **F2 policy.** Is charging 2048 samples when off right, versus using
   `set_latency_samples` with a restart on `dh_on`? This note says
   constant (§5.1). The alternative saves 43 ms on masters that never use
   the stage, at the cost of a dropout on every toggle.
5. **Chain position.** Is a post-saturator position option worth a param
   later? No for v1.
6. **The golden shift** (§5.2). The re-bless is justified, but it touches
   an existing golden. The proof step is mandatory, and it must be
   reported.
7. **Agent visibility.** `max_cut_db` and `cut_curve` exist in the core.
   Should they surface through the plugin viz / a meter field so an agent
   can read "de-harsh cut 4.1 dB at 3.2 kHz"? That is additive, but it
   changes `reading-meters.md` if it goes on the wire. Proposed as a
   follow-up, not W12.
8. **Additive v1.1 candidates:** frequency-dependent attack (soothe:
   "faster on high frequencies", e.g. τ ∝ (1 kHz/f)^½), stereo link %,
   external sidechain key (the compressor pattern), a sensitivity tilt
   (soothe's nodes), and a track-insert wrapper or `eq`-plugin mode with
   the low-latency geometry.
9. **Stale spec text.** §5 of `warmth-width-depth.md` still lists F2 as
   open. Update it when W12 lands (§5.1 here).

## 8. Sources

- oeksound, *soothe2 manual*: https://oeksound.com/manuals/soothe2/
  (control semantics, the frequency-dependent attack remark, and soft/hard
  level dependence).
- J. B. Allen, "Short term spectral analysis, synthesis, and modification
  by discrete Fourier transform", IEEE Trans. ASSP 25(3), 1977; R. E.
  Crochiere, "A weighted overlap-add method of short-time Fourier
  analysis/synthesis", IEEE Trans. ASSP 28(1), 1980 (WOLA, COLA).
- O. Cappé, "Elimination of the musical noise phenomenon with the Ephraim
  and Malah noise suppressor", IEEE Trans. SAP 2(2), 1994 (gain smoothing
  against musical noise).
- P. D. Hatziantoniou, J. N. Mourjopoulos, "Generalized fractional-octave
  smoothing of audio and acoustic responses", JAES 48(4), 2000.
- U. Zölzer (ed.), *DAFX: Digital Audio Effects*, 2nd ed., Wiley 2011,
  ch. 7–8 (spectral processing, cepstral envelopes).
- B. R. Glasberg, B. C. J. Moore, "Derivation of auditory filter shapes
  from notched-noise data", Hearing Research 47, 1990 (ERB; the filterbank
  alternative).
- J. C. Brown, "Calculation of a constant Q spectral transform", JASA
  89(1), 1991 (the constant-Q alternative).
