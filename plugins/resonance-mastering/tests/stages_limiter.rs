use resonance_mastering::stages::limiter::{Limiter, LimiterConfig};
use resonance_metering::true_peak::polyphase::PolyphasePeakDetector;

#[test]
fn disabled_is_pure_delay() {
    let sr = 48_000.0_f32;
    let mut lim = Limiter::new(sr);
    let la = lim.latency();
    let n = la + 1024;
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    for i in 0..n {
        let s = (i as f32 * 0.05).sin() * 0.4;
        l[i] = s;
        r[i] = s;
    }
    let input = l.clone();
    lim.process_stereo(&mut l, &mut r, &LimiterConfig::default());
    for i in la..n {
        assert!((l[i] - input[i - la]).abs() < 1e-6);
    }
}

#[test]
fn quiet_signal_passes_unchanged_when_enabled() {
    let sr = 48_000.0_f32;
    let mut lim = Limiter::new(sr);
    let la = lim.latency();
    let n = la + 1024;
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    for i in 0..n {
        let s = (i as f32 * 0.02).sin() * 0.25; // −12 dBFS, far below ceiling
        l[i] = s;
        r[i] = s;
    }
    let input = l.clone();
    let cfg = LimiterConfig {
        enabled: true,
        ceiling_db: -0.3,
        release_ms: 50.0,
    };
    lim.process_stereo(&mut l, &mut r, &cfg);
    let mut max_err = 0.0_f32;
    for i in la..n {
        max_err = max_err.max((l[i] - input[i - la]).abs());
    }
    assert!(max_err < 1e-5, "quiet sine error = {max_err}");
}

#[test]
fn loud_signal_never_exceeds_ceiling() {
    // Hot 1 kHz sine at -1 dBFS → peaks just under 0 dBFS → limiter
    // clamps it to the ceiling. Output peak must stay at or below
    // the ceiling after the initial delay has settled.
    let sr = 48_000.0_f32;
    let mut lim = Limiter::new(sr);
    let la = lim.latency();
    let n = la + 8192;
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    let amp = 10.0_f32.powf(-1.0 / 20.0); // -1 dBFS
    for i in 0..n {
        let t = i as f32 / sr;
        let s = (std::f32::consts::TAU * 1000.0 * t).sin() * amp;
        l[i] = s;
        r[i] = s;
    }
    let cfg = LimiterConfig {
        enabled: true,
        ceiling_db: -6.0,
        release_ms: 50.0,
    };
    lim.process_stereo(&mut l, &mut r, &cfg);
    let tail_start = la + 2048;
    let peak = l[tail_start..]
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max);
    let ceiling_lin = 10.0_f32.powf(-6.0 / 20.0);
    // Small tolerance for FIR ripple and the release reaching up
    // toward 1.0 briefly between peaks.
    assert!(
        peak <= ceiling_lin * 1.02,
        "output peak {peak} exceeds ceiling {ceiling_lin}"
    );
}

#[test]
fn isolated_intersample_peak_held_to_ceiling() {
    // fs/4 sine sampled at 45° phase offset: every sample sits at
    // ±1/√2 but the true (inter-sample) peak reaches the full
    // amplitude. A short isolated burst of it exercises the detector's
    // group-delay alignment — if the envelope constraint lands late,
    // the burst onset leaks past the ceiling by several tenths of a dB.
    let sr = 48_000.0_f32;
    let mut lim = Limiter::new(sr);
    let la = lim.latency();
    let n = la + 4096;
    let mut l = vec![0.0_f32; n];
    let start = 512;
    for k in 0..256 {
        l[start + k] =
            (std::f32::consts::TAU * 0.25 * k as f32 + std::f32::consts::FRAC_PI_4).sin();
    }
    let mut r = l.clone();
    let cfg = LimiterConfig {
        enabled: true,
        ceiling_db: -1.0,
        release_ms: 50.0,
    };
    lim.process_stereo(&mut l, &mut r, &cfg);
    let mut det = PolyphasePeakDetector::new();
    det.push_block(&l);
    let peak = det.peak();
    let ceiling_lin = 10.0_f32.powf(-1.0 / 20.0);
    let tol = 10.0_f32.powf(0.05 / 20.0); // ≤ 0.05 dB overshoot
    assert!(
        peak <= ceiling_lin * tol,
        "true peak {peak} ({} dBTP) exceeds -1 dBTP ceiling by more than 0.05 dB",
        20.0 * peak.log10()
    );
}

#[test]
fn impulse_never_breaks_ceiling() {
    // A single unit impulse has inter-sample content; the limiter
    // must still keep the oversampled output below its ceiling.
    let sr = 48_000.0_f32;
    let mut lim = Limiter::new(sr);
    let la = lim.latency();
    let n = la + 512;
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    l[64] = 1.0;
    r[64] = 1.0;
    let cfg = LimiterConfig {
        enabled: true,
        ceiling_db: -3.0,
        release_ms: 50.0,
    };
    lim.process_stereo(&mut l, &mut r, &cfg);
    let ceiling_lin = 10.0_f32.powf(-3.0 / 20.0);
    let peak = l.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    assert!(
        peak <= ceiling_lin * 1.02,
        "impulse peak {peak} exceeds ceiling {ceiling_lin}"
    );
}
