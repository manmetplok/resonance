use std::f32::consts::TAU;

use resonance_amp::tuner::{Tuner, FRAME_LEN};

/// Drive the tuner with a pure sine and verify it locks on.
fn detect_sine(sample_rate: f32, hz: f32, total_samples: usize) -> (f32, f32) {
    let mut tuner = Tuner::new(sample_rate);
    let mut buf = vec![0.0f32; 256];
    let mut result = (0.0, 0.0);
    let mut n = 0usize;
    while n < total_samples {
        for (i, s) in buf.iter_mut().enumerate() {
            let t = (n + i) as f32 / sample_rate;
            *s = (TAU * hz * t).sin();
        }
        tuner.feed(&buf);
        if let Some(r) = tuner.analyze() {
            result = r;
        }
        n += buf.len();
    }
    result
}

#[test]
fn detects_a4() {
    let (hz, conf) = detect_sine(48_000.0, 440.0, 8192);
    assert!((hz - 440.0).abs() < 1.0, "A4: got {hz} Hz");
    assert!(conf > 0.8, "A4 confidence too low: {conf}");
}

#[test]
fn detects_low_e() {
    // Low-E guitar string = 82.407 Hz.
    let (hz, conf) = detect_sine(48_000.0, 82.407, 8192);
    assert!((hz - 82.407).abs() < 1.0, "low E: got {hz} Hz");
    assert!(conf > 0.8, "low E confidence too low: {conf}");
}

#[test]
fn detects_high_e() {
    // High-E guitar string = 329.628 Hz.
    let (hz, conf) = detect_sine(48_000.0, 329.628, 8192);
    assert!((hz - 329.628).abs() < 1.5, "high E: got {hz} Hz");
    assert!(conf > 0.8, "high E confidence too low: {conf}");
}

#[test]
fn silence_reports_nothing() {
    let mut tuner = Tuner::new(48_000.0);
    let silence = vec![0.0f32; FRAME_LEN * 2];
    tuner.feed(&silence);
    assert!(tuner.analyze().is_none());
}
