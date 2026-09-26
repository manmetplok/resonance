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

/// A guitar-like harmonic tone: fundamental plus partials 2..5 at
/// falling level.
fn harmonic(hz: f32, sr: f32, n: usize) -> f32 {
    let t = n as f64 / sr as f64;
    (1..=5)
        .map(|k| ((std::f64::consts::TAU * hz as f64 * k as f64 * t).sin() / k as f64) as f32)
        .sum::<f32>()
        * 0.3
}

/// Drive the tuner block by block through `feed_stereo` and return the
/// last reading.
fn detect_stereo(sr: f32, hz: f32, seconds: f32, left_gain: f32, right_gain: f32) -> (f32, f32) {
    let mut tuner = Tuner::new(sr);
    let total = (seconds * sr) as usize;
    let (mut l, mut r) = (vec![0.0f32; 256], vec![0.0f32; 256]);
    let mut result = (0.0, 0.0);
    let mut n = 0usize;
    while n < total {
        for i in 0..256 {
            let s = harmonic(hz, sr, n + i);
            l[i] = s * left_gain;
            r[i] = s * right_gain;
        }
        tuner.feed_stereo(&l, &r);
        if let Some(res) = tuner.analyze() {
            result = res;
        }
        n += 256;
    }
    result
}

/// DSP-11: the lag range used to be clamped to FRAME_LEN/2 samples, so
/// at 96/192 kHz the low strings fell outside it. The tracker now runs
/// on a decimated stream, so its reach in Hz is rate-independent.
#[test]
fn detects_low_e_at_high_sample_rates() {
    for sr in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let (hz, conf) = detect_stereo(sr, 82.407, 1.0, 1.0, 1.0);
        assert!((hz - 82.407).abs() < 0.5, "low E at {sr} Hz: got {hz} Hz");
        assert!(conf > 0.8, "low E at {sr} Hz: confidence {conf}");
    }
    // The A string at 192 kHz (period 1745 samples) was out of reach too.
    let (hz, _) = detect_stereo(192_000.0, 110.0, 1.0, 1.0, 1.0);
    assert!((hz - 110.0).abs() < 0.5, "A at 192k: got {hz} Hz");
}

/// DSP-11: the tuner hears the mono sum, not the left channel only — a
/// guitar on the right input still tunes.
#[test]
fn listens_to_both_channels() {
    let (hz, conf) = detect_stereo(48_000.0, 146.83, 0.5, 0.0, 1.0);
    assert!((hz - 146.83).abs() < 0.5, "right-only D: got {hz} Hz");
    assert!(conf > 0.8, "right-only D: confidence {conf}");
}
