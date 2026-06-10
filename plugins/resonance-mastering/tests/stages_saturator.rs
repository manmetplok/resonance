use resonance_mastering::stages::saturator::{Saturator, SaturatorConfig, Shaper};

#[test]
fn disabled_passes_audio_unchanged() {
    let mut s = Saturator::new(48_000.0);
    let mut left = vec![0.3, -0.4, 0.5, -0.6];
    let mut right = left.clone();
    let expected = left.clone();
    s.process_stereo(&mut left, &mut right, &SaturatorConfig::default());
    assert_eq!(left, expected);
    assert_eq!(right, expected);
}

#[test]
fn waveshaper_clamps_loud_input() {
    // With heavy drive a 1.0-amplitude sine should stay near unity:
    // the shaper is bounded, peak-normalization pins it to 1.0, and
    // only the post-shape +2 dB LF shelf can nudge it slightly over.
    let mut s = Saturator::new(48_000.0);
    let cfg = SaturatorConfig {
        enabled: true,
        drive_db: 12.0,
        character: 0.0,
        mix: 1.0,
        shaper: Shaper::Smooth,
    };
    let n = 1024;
    let mut left = vec![0.0_f32; n];
    let mut right = vec![0.0_f32; n];
    for i in 0..n {
        let s = (i as f32 * 0.05).sin();
        left[i] = s;
        right[i] = s;
    }
    s.process_stereo(&mut left, &mut right, &cfg);
    let peak = left.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    assert!(peak <= 1.30, "peak = {peak}");
}

#[test]
fn heavy_drive_introduces_distortion_harmonics() {
    // Feed a pure sine at f0 through the saturator with heavy drive
    // and confirm the output contains energy at 3*f0 (which the
    // clean input does not).
    let sr = 48_000.0_f32;
    let f0 = 1000.0_f32;
    let mut s = Saturator::new(sr);
    let cfg = SaturatorConfig {
        enabled: true,
        drive_db: 12.0,
        character: 0.0,
        mix: 1.0,
        shaper: Shaper::Smooth,
    };
    let n = 4096;
    let mut left = vec![0.0_f32; n];
    let mut right = vec![0.0_f32; n];
    for i in 0..n {
        let t = i as f32 / sr;
        let x = (std::f32::consts::TAU * f0 * t).sin() * 0.7;
        left[i] = x;
        right[i] = x;
    }
    s.process_stereo(&mut left, &mut right, &cfg);

    // Simple third-harmonic energy detector: correlate with cos(3*f0).
    let mut energy_h3 = 0.0_f32;
    for (i, &sample) in left.iter().enumerate().take(n) {
        let t = i as f32 / sr;
        let basis = (std::f32::consts::TAU * 3.0 * f0 * t).sin();
        energy_h3 += sample * basis;
    }
    energy_h3 = energy_h3.abs() / (n as f32);
    assert!(energy_h3 > 0.01, "h3 energy = {energy_h3}");
}

#[test]
fn asymmetric_saturation_has_no_dc_offset() {
    // Fully asymmetric shaping has a transfer curve with nonzero mean;
    // the post-shaper DC blocker must strip that offset before the LF
    // shelf can amplify it.
    let sr = 48_000.0_f32;
    let f0 = 750.0_f32; // 64 samples per period
    let mut s = Saturator::new(sr);
    let cfg = SaturatorConfig {
        enabled: true,
        drive_db: 12.0,
        character: 1.0,
        mix: 1.0,
        shaper: Shaper::Smooth,
    };
    let n = 16_384;
    let mut left = vec![0.0_f32; n];
    let mut right = vec![0.0_f32; n];
    for i in 0..n {
        let t = i as f32 / sr;
        let x = (std::f32::consts::TAU * f0 * t).sin() * 0.7;
        left[i] = x;
        right[i] = x;
    }
    s.process_stereo(&mut left, &mut right, &cfg);

    // Average over the trailing whole periods, well past the blocker's
    // ~200-sample time constant.
    let tail = &left[n - 8192..];
    let mean = tail.iter().sum::<f32>() / tail.len() as f32;
    assert!(mean.abs() < 1e-3, "mean = {mean}");
}
