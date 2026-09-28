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
        ..LimiterConfig::default()
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
        ..LimiterConfig::default()
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
        ..LimiterConfig::default()
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
        ..LimiterConfig::default()
    };
    lim.process_stereo(&mut l, &mut r, &cfg);
    let ceiling_lin = 10.0_f32.powf(-3.0 / 20.0);
    let peak = l.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    assert!(
        peak <= ceiling_lin * 1.02,
        "impulse peak {peak} exceeds ceiling {ceiling_lin}"
    );
}

/// dB of peak reduction the clipper and the limiter each take from a
/// full-scale 200 Hz tone, clipper (drive 2 dB, hard) into the limiter
/// (ceiling −1 dBTP) with `input_gain_db` of push between them.
fn clip_then_limit(input_gain_db: f32) -> (f32, f32) {
    use resonance_mastering::stages::clipper::{Clipper, ClipperConfig};
    let sr = 48_000.0_f32;
    let n = 48_000;
    let tone: Vec<f32> = (0..n)
        .map(|i| (std::f32::consts::TAU * 200.0 * i as f32 / sr).sin())
        .collect();
    let (mut l, mut r) = (tone.clone(), tone.clone());
    let mut clipper = Clipper::new(sr);
    let clip = ClipperConfig {
        enabled: true,
        drive_db: 2.0,
        softness: 0.0,
    };
    let mut lim = Limiter::new(sr);
    let cfg = LimiterConfig {
        enabled: true,
        ceiling_db: -1.0,
        release_ms: 50.0,
        input_gain_db,
    };
    let peak = |x: &[f32]| x[24_000..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let db = |x: f32| 20.0 * x.log10();
    let mut clipped = Vec::with_capacity(n);
    for start in (0..n).step_by(512) {
        let end = (start + 512).min(n);
        clipper.process_stereo(&mut l[start..end], &mut r[start..end], &clip);
        clipped.extend_from_slice(&l[start..end]);
        lim.process_stereo(&mut l[start..end], &mut r[start..end], &cfg);
    }
    let into_limiter = peak(&clipped) * 10f32.powf(input_gain_db / 20.0);
    let clip_share = db(peak(&tone)) - db(peak(&clipped));
    let lim_share = db(into_limiter) - db(peak(&l));
    (clip_share, lim_share)
}

/// The clipper's ceiling is at −drive dBFS and it is level-matched, so
/// without a push after it the limiter had only |ceiling| − drive dB
/// left (none here: −2 dBFS peaks under a −1 dBTP ceiling). `lim_gain`
/// drives the limiter, and each stage takes its own share: the clipper
/// its 2 dB, the limiter the 5 dB the 6 dB push puts over its ceiling.
#[test]
fn input_gain_splits_the_peak_reduction_between_clipper_and_limiter() {
    let (clip, lim) = clip_then_limit(0.0);
    assert!((clip - 2.0).abs() < 0.3, "clipper took {clip:.2} dB");
    assert!(lim < 0.05, "with no push the limiter should have nothing to do, took {lim:.2} dB");

    let (clip, lim) = clip_then_limit(6.0);
    assert!((clip - 2.0).abs() < 0.3, "clipper took {clip:.2} dB");
    assert!((lim - 5.0).abs() < 0.3, "limiter took {lim:.2} dB, want 5");
}

/// 0 dB of input gain is bit-transparent: the default config and an
/// explicit 0 dB render the same bits.
#[test]
fn zero_input_gain_is_bit_transparent() {
    let sr = 48_000.0_f32;
    let render = |cfg: LimiterConfig| {
        let mut lim = Limiter::new(sr);
        let mut l: Vec<f32> = (0..4096).map(|i| (i as f32 * 0.03).sin() * 1.3).collect();
        let mut r = l.clone();
        lim.process_stereo(&mut l, &mut r, &cfg);
        l
    };
    let base = LimiterConfig {
        enabled: true,
        ceiling_db: -1.0,
        release_ms: 50.0,
        ..LimiterConfig::default()
    };
    let explicit = LimiterConfig {
        input_gain_db: 0.0,
        ..base
    };
    let a = render(base);
    let b = render(explicit);
    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
}
