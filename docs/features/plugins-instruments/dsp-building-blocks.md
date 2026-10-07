# DSP building blocks (`resonance-dsp`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: pieces come from several specs, among them
> `docs/history/specs/reverb-algorithms.md` (reverb primitives),
> `docs/history/specs/deharsh-resonance-suppressor.md` (deharsh),
> `docs/history/specs/warmth-width-depth.md` (saturation, tape,
> hysteresis, decorrelation, Doppler detune).

`resonance-dsp` is the shared library of signal-processing primitives. It
has no plugin trait, no GUI and no I/O, and its only dependency is
`rustfft`. Every plugin, the audio engine and the mastering assistant build
on it. ARCHITECTURE.md's rule: a new filter or effect primitive goes here,
not in the first plugin that needs it.

## Behaviour

What the crate offers, grouped by use (public items from
`resonance-dsp/src/lib.rs`):

| Group | Items | Main users |
|---|---|---|
| Filters | `Biquad`/`BiquadCoeffs` (RBJ cookbook), `OnePole`, `DcBlocker`, `eq::BandType` | EQ, compressor, most plugins |
| Delay + interpolation | `DelayLine`; `hermite4`, `lagrange6`, `bspline6`, `BandlimitedReader` and wrapped readers | delay, granular delay, reverb |
| Oversampling | `Halfband`, `Oversampler` (1×/2×/4× polyphase IIR), `halfband_coefs` | color, mastering, wavetable |
| Saturation + analog | `saturate::{Curve, Adaa1}` (first-order antiderivative anti-aliasing), `tanh_fast`, `transformer::{EmphasisPair, LfWeightedDrive}`, `tape::{HeadBump, HfLoss, Flutter}`, `Hysteresis` (Jiles–Atherton) | color, gate, mastering |
| Dynamics | `dynamics::{Ballistics, soft_knee_gain_reduction_db}`, `BlockPeaks` | compressor, gate, mastering |
| Stereo | `ms_encode`/`ms_decode`, `apply_width`, `apply_balance`, `StereoRotation`, `constant_power_pan`, `stereo_balance`, `AllpassDecorrelator`, `VelvetDecorrelator`, `DopplerShifter` | stereo, reverb, mixer |
| Reverb | `reverb::` FDN, matrices, allpasses, dispersive allpass, absorption, shoebox early reflections, smooth random | reverb |
| Convolution | `FftConvolver` (uniformly partitioned overlap-save) | IR, mastering |
| Granular | `GrainEngine`, `GrainParams`, `SchedulerMode`, `InterpQuality`, `MAX_GRAINS` | granular delay, reverb (shimmer) |
| Pitch | `detect_f0`/`YinDetector` (offline f0 for vocal clips), `segment_notes` (f0 → note blobs), `PitchTracker` (real-time), `formant_pitch_shift`/`FormantShifter` | audio engine (vocal tuning), granular delay |
| Time | `TimeStretch`/`StretchAlgorithm` (OLA, WSOLA, phase vocoder), `detect_tempo`, `fold_bpm` | clip warp and tempo detection in the audio engine |
| Spectral cleanup | `ResonanceSuppressor` (deharsh) | mastering |
| Payload swap | `SwapFader` (crossfade state machine for hot-swapping a model, IR or table) | amp, IR, granular delay, wavetable |
| Utilities | `db_to_linear`, `linear_to_db`, `MIN_DB`, `flush_denormals`, `Lfo`, `SimpleRng`, `hann_window`, `WindowMorph` | everywhere |

Rules every primitive follows: no allocation or locking in per-sample or
per-block processing (allocation happens in constructors or `prepare`
calls), deterministic output for a given seed, and sample-rate-aware
coefficients.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Crate root | `resonance-dsp/src/lib.rs` | the public re-exports above |
| Larger modules | `resonance-dsp/src/reverb/{fdn,matrix,allpass,dispersive,absorption,shoebox,smooth_random}.rs`, `resonance-dsp/src/timestretch/{ola,wsola,phase_vocoder}.rs`, `granular.rs`, `deharsh.rs` | |

Invariant: the crate depends on nothing in the workspace and stays free of
CLAP, GUI and plugin types (`tools/arch-invariants`).

## Surfaces

- **GUI / Control API / MCP:** none directly; reached through the plugins
  and the engine features that use it.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| One test binary per primitive family (34 files) | `resonance-dsp/tests/*.rs`, e.g. `biquad.rs`, `saturate.rs`, `tape.rs`, `hysteresis.rs`, `oversample.rs`, `reverb.rs`, `granular*.rs`, `timestretch.rs`, `tempo.rs`, `pitch.rs`, `pitch_rt.rs`, `deharsh.rs`, `deharsh_nonfinite.rs`, `swap_fader.rs`, `convolver.rs` | `./scripts/run-tests.py -p resonance-dsp` |
| Shared spectrum helpers (coherent-sampling sine, amplitude spectrum, harmonic levels) | `resonance-dsp/tests/common/mod.rs` | |

The crate's tests are property and measurement tests (frequency
responses, harmonic levels, aliasing floors, null tests), not golden
files. Goldens live in the plugins that compose these blocks; see
[dsp-test-support.md](dsp-test-support.md). The test-file count is capped
at 34 by `TEST_BINARY_CAPS`.

Not covered: no benches in this crate; CPU cost is measured in plugin
benches (`plugins/resonance-wavetable/benches/synth.rs`,
`plugins/resonance-amp/benches/`, `plugins/resonance-drums/benches/`).

## Decisions and deviations

- `flush_denormals` moved here from `resonance-common` so most plugins do
  not depend on `resonance-common` at all (ARCH-07).
- Primitives that belong to a single feature still live here when they are
  plain DSP (deharsh, granular), so the plugin keeps only policy and UI.

## Known gaps

None tracked for the crate itself.
