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

/// Goertzel magnitude at one bin of an N-point DFT.
fn bin_mag(x: &[f32], bin: usize) -> f64 {
    let n = x.len();
    let w = std::f64::consts::TAU * bin as f64 / n as f64;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, &s) in x.iter().enumerate() {
        let ph = w * i as f64;
        re += s as f64 * ph.cos();
        im += s as f64 * ph.sin();
    }
    (re * re + im * im).sqrt() / n as f64
}

/// The memoryless (pre-ADAA) shaper, kept here as the aliasing
/// reference the antialiased stage is measured against.
fn naive_gritty(x: f64) -> f64 {
    let u = (x / 1.5).clamp(-1.0, 1.0);
    1.5 * (u - u * u * u / 3.0)
}

#[test]
fn adaa_reduces_folded_harmonic_energy() {
    // A sine at bin 427 of a 4096-point window (~5 kHz at 48 kHz)
    // through the Gritty clipper at full drive. Odd harmonics 5, 7, 9,
    // 11 of bin 427 land above Nyquist (bin 2048) and fold back to
    // bins N - m*427 — frequencies that are NOT harmonics of the
    // fundamental, i.e. audible inharmonic aliasing. First-order ADAA
    // must leave substantially less energy there than the memoryless
    // shaper it replaced.
    let sr = 48_000.0_f32;
    let n = 4096usize;
    let k = 427usize; // fundamental bin; odd, so folds never hit harmonics
    let f0 = k as f32 * sr / n as f32;
    let drive_db = 18.0_f32;
    let drive = 10.0f64.powf(drive_db as f64 / 20.0);

    let mut s = Saturator::new(sr);
    let cfg = SaturatorConfig {
        enabled: true,
        drive_db,
        character: 0.0,
        mix: 1.0,
        shaper: Shaper::Gritty,
    };

    // Two windows: the first primes the stage's filters and the ADAA
    // state, only the second is measured (an exact number of periods,
    // so bins don't leak).
    let total = 2 * n;
    let mut left = vec![0.0f32; total];
    let mut right = vec![0.0f32; total];
    for i in 0..total {
        let x = (std::f32::consts::TAU * f0 * (i as f32) / sr).sin() * 0.9;
        left[i] = x;
        right[i] = x;
    }
    // The naive reference: the same driven sine through the memoryless
    // shaper (normalized like the stage). The stage's shelves are
    // linear — they scale bins by fractions of a dB, they cannot move
    // energy between bins — so an order-of-magnitude comparison of the
    // two spectra is fair.
    let naive: Vec<f32> = left
        .iter()
        .map(|&x| (naive_gritty(x as f64 * drive) / naive_gritty(drive)) as f32)
        .collect();

    s.process_stereo(&mut left, &mut right, &cfg);
    let adaa = &left[n..];
    let naive = &naive[n..];

    // Alias bin of harmonic m: reflect m*k back into 0..=N/2.
    let fold = |m: usize| -> usize {
        let b = (m * k) % n;
        if b > n / 2 { n - b } else { b }
    };
    let alias_energy = |sig: &[f32]| -> f64 {
        [5usize, 7, 9, 11]
            .iter()
            .map(|&m| bin_mag(sig, fold(m)).powi(2))
            .sum()
    };

    let e_naive = alias_energy(naive);
    let e_adaa = alias_energy(adaa);
    eprintln!(
        "alias energy: naive {e_naive:.3e}, adaa {e_adaa:.3e} ({:.1} dB drop)",
        10.0 * (e_naive / e_adaa).log10()
    );
    let fund_adaa = bin_mag(adaa, k);
    // Sanity: the stage still saturates (fundamental present, in-band
    // harmonic 3 present) — the aliasing drop must not come from the
    // shaper doing nothing.
    assert!(fund_adaa > 0.1, "fundamental vanished: {fund_adaa}");
    assert!(
        bin_mag(adaa, 3 * k) > 1e-3,
        "third harmonic vanished — shaper inert"
    );
    // Measured 6.3 dB on this material (the aggregate is dominated by
    // the m = 5 partial, which folds to ~23 kHz where first-order ADAA
    // helps least; the lower folds drop far more). Assert 5 dB so the
    // test pins the mechanism, not the last tenth of a dB.
    assert!(
        e_adaa < 0.316 * e_naive,
        "ADAA left {e_adaa:.3e} of folded-harmonic energy vs the memoryless \
         shaper's {e_naive:.3e} — expected at least a 5 dB drop"
    );
}

#[test]
fn adaa_is_finite_at_parameter_extremes() {
    // Full drive, both shapers, both character extremes, on the
    // nastiest input for a divided difference: full-scale alternation
    // (large dx), long constant runs (dx = 0, the ε-fallback), and a
    // near-constant ramp with steps far below ADAA_EPS.
    let sr = 48_000.0_f32;
    for shaper in [Shaper::Smooth, Shaper::Gritty] {
        for character in [0.0f32, 1.0] {
            let mut s = Saturator::new(sr);
            let cfg = SaturatorConfig {
                enabled: true,
                drive_db: 18.0,
                character,
                mix: 1.0,
                shaper,
            };
            let n = 1024usize;
            let mut left = vec![0.0f32; n];
            for (i, v) in left.iter_mut().enumerate() {
                *v = match i {
                    // ±1 alternation: dx of ~16 at full drive.
                    0..=255 => if i % 2 == 0 { 1.0 } else { -1.0 },
                    // Hard constant: dx exactly 0 every sample.
                    256..=511 => 0.5,
                    // Sub-epsilon ramp: dx ≈ 1e-8, must take the
                    // midpoint fallback, never the 0/0 quotient.
                    512..=767 => 0.5 + (i - 512) as f32 * 1e-9,
                    // Silence after loud content.
                    _ => 0.0,
                };
            }
            let mut right = left.clone();
            s.process_stereo(&mut left, &mut right, &cfg);
            assert!(
                left.iter().chain(right.iter()).all(|v| v.is_finite()),
                "non-finite output ({shaper:?}, character {character})"
            );
        }
    }
}

#[test]
fn constant_input_engages_epsilon_fallback_exactly() {
    // On constant input every ADAA step has dx = 0, so every sample
    // must go through the midpoint fallback f((x0+x1)/2) — and after
    // the first sample the midpoint IS the input, so the wet path must
    // equal the memoryless shape of the constant (before the DC
    // blocker strips it). Verify via the stage: the output must start
    // at the shaped constant (DC blocker passes the first samples
    // nearly untouched) and decay toward zero, all finite. A 0/0 NaN
    // in the fallback would poison every sample instead.
    let sr = 48_000.0_f32;
    let mut s = Saturator::new(sr);
    let cfg = SaturatorConfig {
        enabled: true,
        drive_db: 12.0,
        character: 0.0,
        mix: 1.0,
        shaper: Shaper::Gritty,
    };
    let n = 16_384usize;
    let mut left = vec![0.25f32; n];
    let mut right = left.clone();
    s.process_stereo(&mut left, &mut right, &cfg);
    assert!(left.iter().all(|v| v.is_finite()), "NaN from the fallback");
    // Tail is DC-blocked to ~0.
    let tail_mean = left[n - 4096..].iter().sum::<f32>() / 4096.0;
    assert!(tail_mean.abs() < 1e-3, "tail mean = {tail_mean}");
    // But the shaper genuinely fired: early samples carry the shaped
    // DC before the blocker's ~200-sample time constant eats it.
    assert!(left[2].abs() > 0.05, "wet path silent on constant input");
}
