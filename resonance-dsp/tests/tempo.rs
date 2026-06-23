//! Tests for the clip-warp tempo/BPM detector
//! (`resonance_dsp::detect_tempo`, doc #166):
//!
//! * BPM recovered within tolerance from synthetic click trains across a
//!   range of tempi and sample rates;
//! * BPM recovered from a richer "drum loop" of decaying noise bursts;
//! * confidence is high for a steady pulse and low for white noise;
//! * the estimate stays inside the configured band;
//! * `fold_bpm` octave-folds into a target range;
//! * degenerate inputs (empty / too short / silent) return zero, no panic.

use resonance_dsp::{detect_tempo, detect_tempo_default, fold_bpm, TempoConfig};

const SR: f32 = 48_000.0;

/// A click train at `bpm`: a short decaying exponential blip on each beat,
/// `duration_secs` long, silence in between. Models a metronome/kick pulse.
fn click_train(bpm: f32, sample_rate: f32, duration_secs: f32) -> Vec<f32> {
    let total = (sample_rate * duration_secs) as usize;
    let period = (sample_rate * 60.0 / bpm) as usize;
    let blip = (sample_rate * 0.01) as usize; // 10 ms decay
    let mut buf = vec![0.0f32; total];
    let mut beat = 0;
    while beat * period < total {
        let start = beat * period;
        for j in 0..blip {
            if start + j >= total {
                break;
            }
            // Decaying 2 kHz tone so the onset has spectral content.
            let t = j as f32 / sample_rate;
            let env = (-t * 400.0).exp();
            buf[start + j] += env * (std::f32::consts::TAU * 2_000.0 * t).sin();
        }
        beat += 1;
    }
    buf
}

/// Deterministic xorshift noise in [-amp, amp].
fn noise(len: usize, amp: f32, mut seed: u32) -> Vec<f32> {
    (0..len)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            ((seed as f32 / u32::MAX as f32) - 0.5) * 2.0 * amp
        })
        .collect()
}

/// A "drum loop": a 4-on-the-floor pattern of decaying noise bursts (snare/
/// kick-like) on each beat, at `bpm`. Richer onsets than a pure click train.
fn drum_loop(bpm: f32, sample_rate: f32, duration_secs: f32) -> Vec<f32> {
    let total = (sample_rate * duration_secs) as usize;
    let period = (sample_rate * 60.0 / bpm) as usize;
    let burst = (sample_rate * 0.05) as usize; // 50 ms burst
    let src = noise(burst, 1.0, 0x1234_5678);
    let mut buf = vec![0.0f32; total];
    let mut beat = 0;
    while beat * period < total {
        let start = beat * period;
        for j in 0..burst {
            if start + j >= total {
                break;
            }
            let env = (-(j as f32 / sample_rate) * 60.0).exp();
            buf[start + j] += src[j] * env;
        }
        beat += 1;
    }
    buf
}

// ---------------------------------------------------------------------------
// 1. Click-train BPM accuracy
// ---------------------------------------------------------------------------

#[test]
fn click_train_bpm_within_tolerance() {
    for &bpm in &[90.0f32, 100.0, 120.0, 140.0, 160.0] {
        let signal = click_train(bpm, SR, 12.0);
        let est = detect_tempo_default(&signal, SR);
        assert!(
            (est.bpm - bpm).abs() <= 2.0,
            "bpm {bpm}: estimated {} (conf {})",
            est.bpm,
            est.confidence
        );
        assert!(
            est.confidence > 0.3,
            "bpm {bpm}: low confidence {}",
            est.confidence
        );
    }
}

#[test]
fn click_train_bpm_at_44100() {
    let sr = 44_100.0;
    let bpm = 128.0;
    let signal = click_train(bpm, sr, 12.0);
    let est = detect_tempo_default(&signal, sr);
    assert!(
        (est.bpm - bpm).abs() <= 2.0,
        "estimated {} for {bpm} BPM at 44.1k",
        est.bpm
    );
}

// ---------------------------------------------------------------------------
// 2. Richer material
// ---------------------------------------------------------------------------

#[test]
fn drum_loop_bpm_within_tolerance() {
    let bpm = 124.0;
    let signal = drum_loop(bpm, SR, 12.0);
    let est = detect_tempo_default(&signal, SR);
    assert!(
        (est.bpm - bpm).abs() <= 2.0,
        "estimated {} for {bpm} BPM drum loop (conf {})",
        est.bpm,
        est.confidence
    );
}

// ---------------------------------------------------------------------------
// 3. Confidence separates a pulse from noise
// ---------------------------------------------------------------------------

#[test]
fn steady_pulse_more_confident_than_noise() {
    let pulse = click_train(120.0, SR, 12.0);
    let white = noise((SR * 12.0) as usize, 0.5, 0xC0FF_EE00);

    let pulse_est = detect_tempo_default(&pulse, SR);
    let noise_est = detect_tempo_default(&white, SR);

    assert!(
        pulse_est.confidence > noise_est.confidence,
        "pulse conf {} should exceed noise conf {}",
        pulse_est.confidence,
        noise_est.confidence
    );
    assert!(
        pulse_est.confidence > 0.3,
        "pulse confidence too low: {}",
        pulse_est.confidence
    );
}

// ---------------------------------------------------------------------------
// 4. Estimate stays in the configured band
// ---------------------------------------------------------------------------

#[test]
fn estimate_within_configured_band() {
    // A 60 BPM pulse with a band that excludes it (must fold to an in-band
    // multiple via the lag search, not report an out-of-band tempo).
    let signal = click_train(60.0, SR, 16.0);
    let mut config = TempoConfig::new(SR);
    config.min_bpm = 100.0;
    config.max_bpm = 180.0;
    let est = detect_tempo(&signal, config);
    assert!(
        est.bpm >= config.min_bpm && est.bpm <= config.max_bpm,
        "estimate {} outside [{}, {}]",
        est.bpm,
        config.min_bpm,
        config.max_bpm
    );
    // 60 BPM's in-band octave is 120.
    assert!(
        (est.bpm - 120.0).abs() <= 2.0,
        "expected ~120 (octave of 60), got {}",
        est.bpm
    );
}

// ---------------------------------------------------------------------------
// 5. fold_bpm
// ---------------------------------------------------------------------------

#[test]
fn fold_bpm_into_range() {
    assert!((fold_bpm(60.0, 70.0, 180.0) - 120.0).abs() < 1e-3);
    assert!((fold_bpm(200.0, 70.0, 180.0) - 100.0).abs() < 1e-3);
    assert!((fold_bpm(300.0, 70.0, 180.0) - 150.0).abs() < 1e-3);
    // Already in range: unchanged.
    assert!((fold_bpm(128.0, 70.0, 180.0) - 128.0).abs() < 1e-3);
    // The lower edge is inclusive.
    assert!((fold_bpm(70.0, 70.0, 180.0) - 70.0).abs() < 1e-3);
}

#[test]
fn fold_bpm_guards_bad_input() {
    assert_eq!(fold_bpm(0.0, 70.0, 180.0), 0.0);
    assert_eq!(fold_bpm(-120.0, 70.0, 180.0), -120.0);
    assert!(fold_bpm(f32::NAN, 70.0, 180.0).is_nan());
    // Degenerate range returns the input untouched.
    assert_eq!(fold_bpm(120.0, 180.0, 70.0), 120.0);
}

// ---------------------------------------------------------------------------
// 6. Degenerate inputs
// ---------------------------------------------------------------------------

#[test]
fn empty_and_short_inputs_return_zero() {
    let est = detect_tempo_default(&[], SR);
    assert_eq!(est.bpm, 0.0);
    assert_eq!(est.confidence, 0.0);

    let short = vec![0.0f32; 100];
    let est = detect_tempo_default(&short, SR);
    assert_eq!(est.bpm, 0.0);
    assert_eq!(est.confidence, 0.0);
}

#[test]
fn silence_returns_zero() {
    let silent = vec![0.0f32; (SR * 5.0) as usize];
    let est = detect_tempo_default(&silent, SR);
    assert_eq!(est.bpm, 0.0);
    assert_eq!(est.confidence, 0.0);
}
