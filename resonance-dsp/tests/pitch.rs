//! Tests for monophonic f0 (pitch) detection.

use resonance_dsp::{detect_f0, F0Config, YinDetector};
use std::f32::consts::TAU;

const SR: f32 = 48_000.0;

/// Generate a sine of `freq` Hz for `dur_secs` at amplitude `amp`.
fn sine(freq: f32, dur_secs: f32, amp: f32) -> Vec<f32> {
    let n = (SR * dur_secs) as usize;
    (0..n)
        .map(|i| amp * (TAU * freq * i as f32 / SR).sin())
        .collect()
}

/// Mean f0 over the voiced frames.
fn mean_voiced_f0(frames: &[resonance_dsp::F0Frame]) -> f32 {
    let voiced: Vec<f32> = frames
        .iter()
        .filter(|f| f.voiced)
        .map(|f| f0_or(f))
        .collect();
    assert!(!voiced.is_empty(), "expected at least one voiced frame");
    voiced.iter().sum::<f32>() / voiced.len() as f32
}

fn f0_or(f: &resonance_dsp::F0Frame) -> f32 {
    f.f0_hz
}

#[test]
fn detects_pure_sine_pitch() {
    for &freq in &[110.0_f32, 220.0, 440.0, 880.0] {
        let signal = sine(freq, 0.5, 0.5);
        let frames = detect_f0(&signal, F0Config::new(SR));
        let est = mean_voiced_f0(&frames);
        let cents = 1200.0 * (est / freq).log2();
        assert!(
            cents.abs() < 15.0,
            "freq {freq}: estimated {est} Hz ({cents:.1} cents off)"
        );
    }
}

#[test]
fn detects_harmonic_complex_fundamental() {
    // A sawtooth-ish complex: many YIN-tricking harmonics, fundamental 147 Hz.
    let f0 = 147.0;
    let n = (SR * 0.5) as usize;
    let signal: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let mut s = 0.0;
            for h in 1..=8 {
                s += (TAU * f0 * h as f32 * t).sin() / h as f32;
            }
            0.4 * s
        })
        .collect();
    let frames = detect_f0(&signal, F0Config::new(SR));
    let est = mean_voiced_f0(&frames);
    let cents = 1200.0 * (est / f0).log2();
    assert!(
        cents.abs() < 25.0,
        "estimated {est} Hz ({cents:.1} cents off)"
    );
}

#[test]
fn silence_is_unvoiced() {
    let signal = vec![0.0_f32; (SR * 0.3) as usize];
    let frames = detect_f0(&signal, F0Config::new(SR));
    assert!(!frames.is_empty());
    assert!(
        frames.iter().all(|f| !f.voiced && f.f0_hz == 0.0),
        "silence must not be voiced"
    );
}

#[test]
fn white_noise_is_mostly_unvoiced() {
    // Deterministic LCG noise: aperiodic, should rarely register as voiced.
    let mut state = 0x1234_5678_u32;
    let n = (SR * 0.3) as usize;
    let signal: Vec<f32> = (0..n)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0
        })
        .collect();
    let frames = detect_f0(&signal, F0Config::new(SR));
    let voiced = frames.iter().filter(|f| f.voiced).count();
    assert!(
        voiced as f32 / (frames.len() as f32) < 0.2,
        "noise produced too many voiced frames: {voiced}/{}",
        frames.len()
    );
}

#[test]
fn voiced_frames_have_high_confidence() {
    let signal = sine(220.0, 0.4, 0.5);
    let frames = detect_f0(&signal, F0Config::new(SR));
    for f in frames.iter().filter(|f| f.voiced) {
        assert!(
            f.confidence >= 0.5 && f.confidence <= 1.0,
            "voiced confidence out of range: {}",
            f.confidence
        );
    }
}

#[test]
fn short_input_returns_empty() {
    let cfg = F0Config::new(SR);
    let signal = vec![0.1_f32; cfg.frame_size - 1];
    assert!(detect_f0(&signal, cfg).is_empty());
}

#[test]
fn frame_times_are_monotonic_and_centred() {
    let cfg = F0Config::new(SR);
    let signal = sine(200.0, 0.5, 0.4);
    let frames = detect_f0(&signal, cfg);
    assert!(frames.len() >= 2);
    // First frame centred on frame_size/2.
    let expected_first = cfg.frame_size as f32 * 0.5 / SR;
    assert!((frames[0].time_secs - expected_first).abs() < 1e-6);
    // Strictly increasing by hop/sr.
    let step = cfg.hop_size as f32 / SR;
    for w in frames.windows(2) {
        let dt = w[1].time_secs - w[0].time_secs;
        assert!((dt - step).abs() < 1e-4, "non-uniform spacing: {dt}");
    }
}

#[test]
fn reused_detector_matches_oneshot() {
    let cfg = F0Config::new(SR);
    let signal = sine(330.0, 0.3, 0.5);
    let oneshot = detect_f0(&signal, cfg);
    let mut det = YinDetector::new(cfg);
    let reused_a = det.analyze(&signal);
    let reused_b = det.analyze(&signal);
    assert_eq!(oneshot, reused_a);
    assert_eq!(reused_a, reused_b, "reuse must be deterministic");
}

#[test]
fn tracks_pitch_step() {
    // 200 Hz then 400 Hz; voiced frames in each half should track.
    let mut signal = sine(200.0, 0.4, 0.5);
    signal.extend(sine(400.0, 0.4, 0.5));
    let frames = detect_f0(&signal, F0Config::new(SR));
    let mid = 0.4_f32;
    let low: Vec<f32> = frames
        .iter()
        .filter(|f| f.voiced && f.time_secs < mid - 0.05)
        .map(|f| f.f0_hz)
        .collect();
    let high: Vec<f32> = frames
        .iter()
        .filter(|f| f.voiced && f.time_secs > mid + 0.05)
        .map(|f| f.f0_hz)
        .collect();
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    assert!((mean(&low) - 200.0).abs() < 5.0, "low half: {}", mean(&low));
    assert!(
        (mean(&high) - 400.0).abs() < 8.0,
        "high half: {}",
        mean(&high)
    );
}

#[test]
fn works_at_44100() {
    let sr = 44_100.0;
    let n = (sr * 0.4) as usize;
    let freq = 261.63; // middle C
    let signal: Vec<f32> = (0..n)
        .map(|i| 0.5 * (TAU * freq * i as f32 / sr).sin())
        .collect();
    let frames = detect_f0(&signal, F0Config::new(sr));
    let voiced: Vec<f32> = frames
        .iter()
        .filter(|f| f.voiced)
        .map(|f| f.f0_hz)
        .collect();
    assert!(!voiced.is_empty());
    let est = voiced.iter().sum::<f32>() / voiced.len() as f32;
    let cents = 1200.0 * (est / freq).log2();
    assert!(
        cents.abs() < 15.0,
        "estimated {est} Hz ({cents:.1} cents off)"
    );
}
