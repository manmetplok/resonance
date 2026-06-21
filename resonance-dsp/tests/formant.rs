//! Tests for the formant-preserving pitch-shift primitive
//! (`resonance_dsp::FormantShifter`, doc #160 / todo #353):
//!
//! * a unit ratio is a near-identical copy of the input;
//! * output length always equals input length (timing untouched);
//! * the fundamental moves by the requested ratio (±12 semitones), read
//!   back with the crate's YIN detector;
//! * the spectral envelope (formants) of a synthetic vowel stays put when
//!   the pitch is shifted — unlike a naive resample, which drags the
//!   formants along (the "chipmunk" effect this primitive avoids);
//! * a time-varying ratio curve sweeps the pitch across the clip;
//! * output stays finite and stereo channels are handled.

use resonance_dsp::{detect_f0, formant_pitch_shift, F0Config, FormantShifter};

const SR: f32 = 48_000.0;

/// Frequency multiplier for a semitone offset.
fn ratio_of(semitones: f32) -> f32 {
    2f32.powf(semitones / 12.0)
}

/// A pure sine of `freq` Hz, `len` samples, amplitude 0.4.
fn sine(freq: f32, len: usize) -> Vec<f32> {
    use std::f32::consts::TAU;
    (0..len)
        .map(|i| 0.4 * (TAU * freq * i as f32 / SR).sin())
        .collect()
}

/// Magnitude of a single resonance (formant) at `f`, a Gaussian bump
/// centred on `centre` Hz with width `bw` Hz and linear gain `gain`.
fn formant_gain(f: f32, centre: f32, bw: f32, gain: f32) -> f32 {
    let z = (f - centre) / bw;
    gain * (-0.5 * z * z).exp()
}

/// A synthetic vowel: a harmonic series at `f0` whose harmonics are
/// shaped by two formant resonances (≈ an "ah": F1 730 Hz, F2 1090 Hz).
/// This is the kind of signal whose timbre lives in its formant envelope.
fn vowel(f0: f32, len: usize) -> Vec<f32> {
    use std::f32::consts::TAU;
    let mut out = vec![0.0f32; len];
    let mut h = 1;
    loop {
        let f = f0 * h as f32;
        if f > 5_000.0 {
            break;
        }
        let amp = formant_gain(f, 730.0, 130.0, 1.0)
            + formant_gain(f, 1_090.0, 180.0, 0.7)
            + 0.02; // a little broadband so high harmonics never fully vanish
        for (i, s) in out.iter_mut().enumerate() {
            *s += amp * (TAU * f * i as f32 / SR).sin();
        }
        h += 1;
    }
    // Normalise to a comfortable peak.
    let peak = out.iter().fold(0.0f32, |m, &x| m.max(x.abs())).max(1e-9);
    for s in out.iter_mut() {
        *s *= 0.5 / peak;
    }
    out
}

/// Mean f0 over the voiced frames of `samples`, via the YIN detector.
fn measured_f0(samples: &[f32]) -> f32 {
    let mut config = F0Config::new(SR);
    config.f_min = 80.0;
    config.f_max = 2_000.0;
    let frames = detect_f0(samples, config);
    let voiced: Vec<f32> = frames
        .iter()
        .filter(|f| f.voiced)
        .map(|f| f.f0_hz)
        .collect();
    assert!(!voiced.is_empty(), "no voiced frames detected");
    voiced.iter().sum::<f32>() / voiced.len() as f32
}

/// Spectral centroid (Hz) of `samples` over 0..`f_hi` Hz, a compact proxy
/// for "where the spectral energy sits" — i.e. where the formants are.
/// Uses a Hann-windowed DFT evaluated on a coarse frequency grid, which is
/// plenty to locate a broad formant structure.
fn spectral_centroid(samples: &[f32], f_hi: f32) -> f32 {
    use std::f32::consts::TAU;
    // Analyse a steady central chunk, Hann-windowed to tame leakage.
    let n = samples.len().min(16_384);
    let start = (samples.len() - n) / 2;
    let frame: Vec<f32> = (0..n)
        .map(|i| {
            let w = 0.5 - 0.5 * (TAU * i as f32 / (n as f32 - 1.0)).cos();
            samples[start + i] * w
        })
        .collect();
    let bins = 256usize;
    let mut num = 0.0f32;
    let mut den = 0.0f32;
    for b in 1..bins {
        let f = f_hi * b as f32 / bins as f32;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, &x) in frame.iter().enumerate() {
            let ph = TAU * f * i as f32 / SR;
            re += x * ph.cos();
            im -= x * ph.sin();
        }
        let mag = (re * re + im * im).sqrt();
        num += f * mag;
        den += mag;
    }
    if den > 0.0 {
        num / den
    } else {
        0.0
    }
}

/// Naive (chipmunk) pitch shift: resample by `ratio`, which multiplies
/// every frequency — harmonics *and* formants — by `ratio`.
fn naive_resample(samples: &[f32], ratio: f32) -> Vec<f32> {
    let out_len = (samples.len() as f32 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let x = i as f32 * ratio;
            let j = x.floor() as usize;
            if j + 1 >= samples.len() {
                samples[samples.len() - 1]
            } else {
                let frac = x - j as f32;
                samples[j] + (samples[j + 1] - samples[j]) * frac
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Unity ratio ≈ identity
// ---------------------------------------------------------------------------

/// A unit ratio passes the signal through essentially unchanged: the
/// interior (away from the windowed edges) matches the input closely.
#[test]
fn unity_ratio_is_near_identity() {
    let input = sine(330.0, 24_000);
    let out = formant_pitch_shift(&input, SR, &[1.0]);
    assert_eq!(out.len(), input.len());

    let skip = 4_096; // past the priming edge
    let mut max_err = 0.0f32;
    for i in skip..input.len() - skip {
        max_err = max_err.max((out[i] - input[i]).abs());
    }
    assert!(
        max_err < 1e-3,
        "unity ratio should be near-identity, max interior error {max_err}"
    );
}

/// An empty curve is also treated as no shift.
#[test]
fn empty_curve_is_no_shift() {
    let input = sine(330.0, 12_000);
    let out = formant_pitch_shift(&input, SR, &[]);
    let skip = 4_096;
    for i in skip..input.len() - skip {
        assert!((out[i] - input[i]).abs() < 1e-3);
    }
}

// ---------------------------------------------------------------------------
// 2. Length is preserved (timing untouched)
// ---------------------------------------------------------------------------

/// Output length equals input length for any ratio — the primitive
/// repitches without restretching.
#[test]
fn output_length_matches_input() {
    let shifter = FormantShifter::new(SR);
    let input = sine(440.0, 30_000);
    for &semis in &[-12.0f32, -5.0, 0.0, 7.0, 12.0] {
        let out = shifter.process(&input, &[ratio_of(semis)]);
        assert_eq!(out.len(), input.len(), "length changed at {semis:+} st");
    }
}

// ---------------------------------------------------------------------------
// 3. Pitch tracks the ratio (±12 semitones)
// ---------------------------------------------------------------------------

/// Shifting by `p` semitones multiplies the detected fundamental by
/// `2^(p/12)`. Covers the DoD's ±12-semitone range.
#[test]
fn pitch_shift_moves_fundamental() {
    let base = 220.0f32;
    let input = vowel(base, 48_000);
    let shifter = FormantShifter::new(SR);
    for &semis in &[-12.0f32, -5.0, 7.0, 12.0] {
        let out = shifter.process(&input, &[ratio_of(semis)]);
        let expected = base * ratio_of(semis);
        let got = measured_f0(&out);
        let rel = (got - expected).abs() / expected;
        assert!(
            rel < 0.05,
            "{semis:+} st: detected {got:.1} Hz, expected {expected:.1} Hz (rel {rel:.3})"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Formant envelope is preserved (no chipmunk)
// ---------------------------------------------------------------------------

/// Shifting a vowel up an octave keeps its formant structure roughly in
/// place (spectral centroid barely moves), whereas a naive resample drags
/// the formants up with the pitch (centroid rises sharply). This is the
/// core DoD: ±12 semitones preserves the spectral envelope.
#[test]
fn formant_envelope_is_preserved_under_shift() {
    let input = vowel(160.0, 48_000);
    let shifter = FormantShifter::new(SR);
    let shifted = shifter.process(&input, &[ratio_of(12.0)]);

    let c_in = spectral_centroid(&input, 4_000.0);
    let c_formant = spectral_centroid(&shifted, 4_000.0);
    let c_naive = spectral_centroid(&naive_resample(&input, ratio_of(12.0)), 4_000.0);

    // Formant-preserving: centroid stays near the original.
    let drift = (c_formant - c_in).abs() / c_in;
    assert!(
        drift < 0.20,
        "formant centroid drifted {drift:.2} (in {c_in:.0} Hz → {c_formant:.0} Hz)"
    );
    // Naive resample: centroid climbs toward 2× — the chipmunk effect.
    assert!(
        c_naive > 1.5 * c_in,
        "naive resample should raise the centroid (in {c_in:.0} Hz → {c_naive:.0} Hz)"
    );
    // And the formant-preserving result is markedly closer to the original
    // than the naive one.
    assert!(
        (c_formant - c_in).abs() < 0.5 * (c_naive - c_in).abs(),
        "formant-preserving centroid ({c_formant:.0}) should beat naive ({c_naive:.0}) vs in {c_in:.0}"
    );
}

// ---------------------------------------------------------------------------
// 5. Time-varying curve sweeps the pitch
// ---------------------------------------------------------------------------

/// A ratio curve ramping from unity up an octave sweeps the pitch across
/// the clip: an early window sits near the original pitch and a late one
/// near double, each matching the curve sampled at that window's centre.
/// This is the time-varying behaviour epic #20's warp relies on.
#[test]
fn time_varying_curve_sweeps_pitch() {
    let base = 220.0f32;
    let input = vowel(base, 60_000);
    let curve = [1.0f32, ratio_of(12.0)]; // linear ramp unity → +12 st
    let out = formant_pitch_shift(&input, SR, &curve);

    // The curve is linear, so its mean over a window equals its value at
    // the window centre: ratio(t) = 1 + t·(2 − 1) = 1 + t.
    let win = out.len() / 8;
    let f_start = measured_f0(&out[..win]);
    let f_end = measured_f0(&out[out.len() - win..]);
    let expect_start = base * (1.0 + 1.0 / 16.0); // window centred at t ≈ 1/16
    let expect_end = base * (1.0 + 15.0 / 16.0); // window centred at t ≈ 15/16

    assert!(
        (f_start - expect_start).abs() / expect_start < 0.06,
        "clip start should be ~{expect_start:.1} Hz, got {f_start:.1}"
    );
    assert!(
        (f_end - expect_end).abs() / expect_end < 0.06,
        "clip end should be ~{expect_end:.1} Hz, got {f_end:.1}"
    );
    assert!(f_end > 1.5 * f_start, "pitch should rise across the clip");
}

// ---------------------------------------------------------------------------
// 6. Robustness: finite output, clamping, stereo
// ---------------------------------------------------------------------------

/// Out-of-range and non-finite ratios are clamped / sanitised and never
/// produce NaNs or infinities.
#[test]
fn output_is_finite_under_extreme_curves() {
    let input = vowel(200.0, 16_000);
    let out = formant_pitch_shift(&input, SR, &[100.0, f32::NAN, -3.0, 0.0]);
    assert_eq!(out.len(), input.len());
    assert!(out.iter().all(|s| s.is_finite()), "non-finite sample");
}

/// Stereo processing shifts both channels and keeps their lengths.
#[test]
fn stereo_shifts_both_channels() {
    let left = vowel(180.0, 24_000);
    let right = sine(180.0, 24_000);
    let shifter = FormantShifter::new(SR);
    let (l, r) = shifter.process_stereo(&left, &right, &[ratio_of(5.0)]);
    assert_eq!(l.len(), left.len());
    assert_eq!(r.len(), right.len());
    assert!(l.iter().chain(r.iter()).all(|s| s.is_finite()));
    let expected = 180.0 * ratio_of(5.0);
    let got = measured_f0(&r);
    assert!(
        (got - expected).abs() / expected < 0.05,
        "right channel pitch {got:.1} Hz, expected {expected:.1} Hz"
    );
}

/// An empty input yields an empty output rather than panicking.
#[test]
fn empty_input_is_empty_output() {
    let shifter = FormantShifter::new(SR);
    assert!(shifter.process(&[], &[2.0]).is_empty());
}
