# Metering library (`resonance-metering`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: measurement pieces from
> `docs/history/specs/warmth-width-depth.md` (probe, detail proxies) and
> `docs/history/specs/reverb-algorithms.md` (decay analysis).

`resonance-metering` holds every measurement the project makes: loudness,
true peak, loudness range, spectrum, correlation, crest and PLR, offline
whole-buffer analysis for mixing, decay analysis for reverbs, the
insert-chain probe and the `meter.*` detail proxies. It is framework-free
(depends on `resonance-dsp`, `rustfft` and `arc-swap`) and is used by 11 of
the 13 plugins (all but drums and wavetable), the mastering assistant, the
audio engine and the app.

## Behaviour

- **Loudness to ITU-R BS.1770-4 / EBU R128.** `KWeightingFilter`,
  `LufsMeter` (momentary, short-term, gated integrated, plus
  `analyze_offline`), `LraMeter` (EBU loudness range, with a real-time
  variant), `TruePeakMeter` (4× polyphase oversampled inter-sample peak).
- **Other meters.** `CorrelationMeter`, `CrestMeter`, `PlrMeter` /
  `RangeDynamics`.
- **Spectrum.** `SpectrumAnalyzer`/`SpectrumHandle` run the FFT on a
  background thread fed through a ring; `FFT_SIZE`, octave bins
  (`NUM_OCTAVE_BINS`), offline LTAS (`spectrum::offline`, for example
  `sixth_octave_ltas`).
- **Publishing to a UI.** `MeterSnapshot` aggregates readings;
  `AtomicMeterSnapshot` and `viz::{AtomicF32, AtomicF32Pair,
  AtomicF32Array, AtomicHistoryRing}` move values from the audio thread to
  an editor or the app without locks.
- **Offline mix analysis** (`offline`): `band_shares`, `mono_penalty_db`,
  `sample_peak_db`, `clipped_samples`, `range_correlation`,
  `range_crest_db`.
- **Decay analysis** (`decay`): energy decay curves, T20/T30/EDT per
  octave band, echo density, modal peakiness, IACC, and `program_decay` for
  reverb on programme material.
- **Probe** (`probe`): bin-exact stimulus generation (`probe_sine`,
  `smpte_pair`) and harmonic-signature reading for the insert-chain probe.
- **Detail proxies** (`detail`): spectrum (tilt, 1/3-octave LTAS,
  centroid, resonances), stereo (per-band correlation, S/M, mono loss,
  balance, one-sidedness, Haas lag) and depth (HF tilt and related ranking
  estimates), behind the opt-in `meter.*` detail fields.

Real-time meters do no allocation or locking in `process`; spectrum work
happens on its own thread.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Loudness | `resonance-metering/src/{k_weighting,lra}.rs`, `lufs/{mod,gating,incremental,integrated,block_accumulator}.rs` | `LufsMeter`, `LufsReadout`, `LraMeter` |
| True peak | `src/true_peak/{mod,polyphase,coefficients}.rs` | `TruePeakMeter` |
| Spectrum | `src/spectrum/{mod,ring,fft_worker,octave,offline}.rs` | `SpectrumAnalyzer`, `SpectrumSnapshot` |
| Snapshots + viz | `src/{snapshot,atomic_snapshot,viz}.rs` | `MeterSnapshot`, `AtomicMeterSnapshot` |
| Offline, decay, probe, detail | `src/offline.rs`, `src/decay/`, `src/probe.rs`, `src/detail/{spectrum,stereo,depth}.rs` | `BandShares`, `ProgramDecay`, `SpectrumDetail` |

## Surfaces

- **GUI:** every plugin meter and analyser and the reference A/B view
  ([reference-ab-monitoring.md](../mixer/reference-ab-monitoring.md), via
  `MeterSnapshot`). The mixer's strip meters are plain sample peaks
  computed in the engine, not this crate
  ([mixer/metering.md](../mixer/metering.md)).
- **Control API / MCP:** indirectly through the meter tools
  (`mcp__resonance__meter_snapshot`, `meter_measure`, `meter_compare`,
  `meter_probe`, `meter_stems`) and `master_assist`; see
  [meter-control-api.md](../mixer/meter-control-api.md) and
  [mastering-assist.md](../plugins-effects/mastering-assist.md).
- **Agent skills:** `mixing`, `mastering` and `spatial` read these numbers
  through the meter tools.

## How it's tested

| What | Where | Run |
|---|---|---|
| Standards compliance | `resonance-metering/tests/{ebu_3341,ebu_3342}.rs` (subset of EBU test cases, ±0.1 LU), `k_weighting.rs`, `lufs_*.rs`, `true_peak*.rs`, `lra*.rs`, `plr.rs` | `./scripts/run-tests.py -p resonance-metering` |
| Spectrum + viz | `tests/{spectrum_ring,spectrum_octave,spectrum_reset,viz_atomics}.rs` | same |
| Analysis | `tests/{offline_levels,offline_band_shares,decay,probe,detail_spectrum,detail_stereo,detail_depth,correlation,crest}.rs` | same |

26 test files (capped by `TEST_BINARY_CAPS`); no goldens. Not covered: the
EBU 3341 cases for 5.1 material (the crate meters stereo only).

## Decisions and deviations

- Measurement lives in one crate so the mastering plugin, the mastering
  assistant and the app's `meter.*` methods agree on every number.

## Known gaps

None tracked for the crate itself.
