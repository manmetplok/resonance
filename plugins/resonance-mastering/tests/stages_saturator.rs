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

#[test]
fn toggle_mid_signal_does_not_click() {
    // Toggling the saturator off used to hard-switch from the wet
    // signal (saturated + LF-shelf-boosted) straight to dry in one
    // sample. The enable crossfade must keep the sample-to-sample
    // slope in the same class as a steady run.
    let sr = 48_000.0_f32;
    let cfg_on = SaturatorConfig {
        enabled: true,
        drive_db: 6.0,
        character: 0.0,
        mix: 1.0,
        shaper: Shaper::Smooth,
    };
    let cfg_off = SaturatorConfig {
        enabled: false,
        ..cfg_on
    };

    // 60 Hz → 800-sample period, so 100-frame blocks give toggle
    // boundaries every 45° of the cycle. The wet path phase-shifts
    // the sine (DC blocker + shelves), so the phase where the wet/dry
    // step is worst is not obvious a priori — try a whole period of
    // toggle positions and take the worst.
    let block = 100;
    let total = 14_000;
    let input: Vec<f32> = (0..total)
        .map(|i| (i as f32 / sr * 60.0 * std::f32::consts::TAU).sin() * 0.9)
        .collect();

    let render = |toggle_at: Option<usize>| -> Vec<f32> {
        let mut s = Saturator::new(sr);
        let mut l = input.clone();
        let mut r = input.clone();
        let mut start = 0;
        while start < total {
            let end = (start + block).min(total);
            let cfg = match toggle_at {
                Some(t) if start >= t => &cfg_off,
                _ => &cfg_on,
            };
            s.process_stereo(&mut l[start..end], &mut r[start..end], cfg);
            start = end;
        }
        l
    };

    let max_delta = |x: &[f32], around: usize| {
        x[around - 400..around + 1600]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0_f32, f32::max)
    };

    let steady = render(None);
    let mut steady_max = 0.0_f32;
    let mut toggled_max = 0.0_f32;
    for k in 0..8 {
        let toggle = 9800 + k * block;
        let toggled = render(Some(toggle));
        steady_max = steady_max.max(max_delta(&steady, toggle));
        toggled_max = toggled_max.max(max_delta(&toggled, toggle));
    }
    // The old hard toggle stepped by the full wet/dry difference at
    // the worst phase (~0.1 with this drive); the crossfade keeps the
    // toggled run's worst slope within a factor of the steady run's.
    assert!(
        toggled_max < steady_max * 1.5 + 0.01,
        "toggle stepped {toggled_max} per sample vs {steady_max} steady"
    );
}
