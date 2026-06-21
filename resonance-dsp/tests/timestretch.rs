//! Tests for the clip-warp time-stretch + pitch-shift processor
//! (`resonance_dsp::TimeStretch`, doc #166 D1):
//!
//! * output length tracks `time_ratio` for both algorithms;
//! * `pitch_semitones` shifts frequency (±12 covered) at `time_ratio = 1`,
//!   measured back with the crate's YIN detector;
//! * stretching alone leaves pitch unchanged;
//! * block-by-block streaming is bitwise-identical to one offline pass
//!   (the determinism the mixer relies on for live == bounce);
//! * latency is reported.

use resonance_dsp::{detect_f0, F0Config, StretchAlgorithm, TimeStretch};

const SR: f32 = 48_000.0;

/// A pure sine of `freq` Hz, `len` samples, amplitude 0.5.
fn sine(freq: f32, len: usize) -> Vec<f32> {
    use std::f32::consts::TAU;
    (0..len)
        .map(|i| 0.5 * (TAU * freq * i as f32 / SR).sin())
        .collect()
}

/// Deterministic xorshift noise in [-0.5, 0.5].
fn noise(len: usize, mut seed: u32) -> Vec<f32> {
    (0..len)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f32 / u32::MAX as f32) - 0.5
        })
        .collect()
}

/// Mean f0 over the voiced frames of `samples`, using the YIN detector.
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

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

const ALGOS: [StretchAlgorithm; 2] = [StretchAlgorithm::Tonal, StretchAlgorithm::Transient];

// ---------------------------------------------------------------------------
// 1. Length accuracy
// ---------------------------------------------------------------------------

/// Output length ≈ input length × `time_ratio` (pitch held at 0) for a
/// range of ratios and both algorithms. Tolerance allows for the
/// frame-quantised flush tail.
#[test]
fn output_length_tracks_time_ratio() {
    let n = 48_000usize;
    let input = sine(440.0, n);
    for algo in ALGOS {
        for &ratio in &[0.5f32, 0.75, 1.0, 1.5, 2.0] {
            let out = TimeStretch::process(SR, algo, ratio, 0.0, &input);
            let expected = n as f32 * ratio;
            let tol = 0.05 * expected + 2.0 * 1024.0;
            let err = (out.len() as f32 - expected).abs();
            assert!(
                err < tol,
                "{algo:?} ratio {ratio}: got {} samples, expected ~{expected} (err {err} > tol {tol})",
                out.len()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Pitch accuracy (±12 semitones, stretch = 1)
// ---------------------------------------------------------------------------

/// With `time_ratio = 1`, shifting by `p` semitones multiplies the
/// detected frequency by 2^(p/12). Covers the DoD's ±12-semitone range.
#[test]
fn pitch_shift_moves_frequency() {
    let base = 440.0f32;
    let input = sine(base, 48_000);
    for algo in ALGOS {
        for &semis in &[-12.0f32, -7.0, 0.0, 5.0, 12.0] {
            let out = TimeStretch::process(SR, algo, 1.0, semis, &input);
            let expected = base * 2f32.powf(semis / 12.0);
            let got = measured_f0(&out);
            let rel = (got - expected).abs() / expected;
            assert!(
                rel < 0.04,
                "{algo:?} {semis:+} st: detected {got:.1} Hz, expected {expected:.1} Hz (rel {rel:.3})"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Stretching alone preserves pitch
// ---------------------------------------------------------------------------

/// Time-stretching (pitch held at 0) leaves the fundamental unchanged.
#[test]
fn stretch_preserves_pitch() {
    let base = 440.0f32;
    let input = sine(base, 48_000);
    for algo in ALGOS {
        for &ratio in &[0.5f32, 1.5, 2.0] {
            let out = TimeStretch::process(SR, algo, ratio, 0.0, &input);
            let got = measured_f0(&out);
            let rel = (got - base).abs() / base;
            assert!(
                rel < 0.04,
                "{algo:?} ratio {ratio}: pitch drifted to {got:.1} Hz from {base} (rel {rel:.3})"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Output is non-trivial (gain sane)
// ---------------------------------------------------------------------------

/// The processor neither silences nor explodes the signal: output RMS is
/// within a factor of two of the input.
#[test]
fn output_gain_is_sane() {
    let input = sine(440.0, 48_000);
    let in_rms = rms(&input);
    for algo in ALGOS {
        for &(ratio, semis) in &[(1.0f32, 0.0f32), (2.0, 0.0), (1.0, 7.0), (0.5, -5.0)] {
            let out = TimeStretch::process(SR, algo, ratio, semis, &input);
            let out_rms = rms(&out[1024.min(out.len())..]); // skip priming edge
            assert!(
                out_rms > 0.5 * in_rms && out_rms < 2.0 * in_rms,
                "{algo:?} ratio {ratio} {semis:+}st: out RMS {out_rms:.4} vs in {in_rms:.4}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Determinism: streaming == offline (live == bounce)
// ---------------------------------------------------------------------------

/// Feeding the input in odd-sized blocks and pulling in odd-sized blocks
/// produces exactly the same samples, bit for bit, as one offline pass —
/// the guarantee that live playback and offline bounce render identically.
#[test]
fn chunked_streaming_is_bitwise_identical_to_offline() {
    let input = noise(40_000, 0x1234_5678);
    for algo in ALGOS {
        let (ratio, semis) = (1.5f32, 4.0f32);
        let offline = TimeStretch::process(SR, algo, ratio, semis, &input);

        let mut ts = TimeStretch::new(SR, algo);
        ts.set_time_ratio(ratio);
        ts.set_pitch_semitones(semis);

        let mut streamed = Vec::new();
        let mut pull_buf = vec![0.0f32; 333];
        let mut i = 0;
        let feed_chunk = 257;
        while i < input.len() {
            let end = (i + feed_chunk).min(input.len());
            ts.feed(&input[i..end]);
            i = end;
            loop {
                let n = ts.pull(&mut pull_buf);
                if n == 0 {
                    break;
                }
                streamed.extend_from_slice(&pull_buf[..n]);
            }
        }
        ts.finish();
        loop {
            let n = ts.pull(&mut pull_buf);
            if n == 0 {
                break;
            }
            streamed.extend_from_slice(&pull_buf[..n]);
        }

        assert_eq!(
            streamed.len(),
            offline.len(),
            "{algo:?}: streamed {} vs offline {} samples",
            streamed.len(),
            offline.len()
        );
        for (k, (a, b)) in streamed.iter().zip(offline.iter()).enumerate() {
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "{algo:?}: sample {k} differs (streamed {a}, offline {b})"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 6. Latency is reported
// ---------------------------------------------------------------------------

/// Latency is a positive, finite sample count and shrinks as the pitch
/// resample rate rises (output is pulled faster).
#[test]
fn latency_is_reported() {
    let mut ts = TimeStretch::new(SR, StretchAlgorithm::Tonal);
    assert!(ts.latency() > 0, "latency must be positive");
    let base = ts.latency();
    ts.set_pitch_semitones(12.0); // resample rate ×2 → ~half the output latency
    assert!(ts.latency() < base, "higher pitch ratio should report less output latency");
}

// ---------------------------------------------------------------------------
// 7. Parameter clamping / robustness
// ---------------------------------------------------------------------------

/// Non-finite or out-of-range parameters fall back to safe values and
/// never produce NaNs in the output.
#[test]
fn parameters_are_clamped_and_output_is_finite() {
    let input = sine(440.0, 8_000);
    let mut ts = TimeStretch::new(SR, StretchAlgorithm::Transient);
    ts.set_time_ratio(f32::NAN);
    assert_eq!(ts.time_ratio(), 1.0);
    ts.set_time_ratio(1_000.0);
    assert!(ts.time_ratio() <= 10.0);
    ts.set_pitch_semitones(f32::INFINITY);
    assert_eq!(ts.pitch_semitones(), 0.0);

    let out = TimeStretch::process(SR, StretchAlgorithm::Tonal, 2.0, 12.0, &input);
    assert!(out.iter().all(|s| s.is_finite()), "output contains non-finite samples");
}
