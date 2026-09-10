use resonance_mastering::stages::glue_compressor::{GlueCompressor, GlueCompressorConfig};

#[test]
fn disabled_passes_audio_unchanged() {
    let mut c = GlueCompressor::new(48_000.0);
    let mut left = vec![0.5, -0.5, 0.3, -0.7, 0.9, -0.9];
    let mut right = left.clone();
    let expected = left.clone();
    c.process_stereo(&mut left, &mut right, &GlueCompressorConfig::default());
    assert_eq!(left, expected);
    assert_eq!(right, expected);
}

#[test]
fn sub_threshold_signal_is_untouched() {
    let mut c = GlueCompressor::new(48_000.0);
    let cfg = GlueCompressorConfig {
        enabled: true,
        threshold_db: -6.0,
        knee_db: 0.0,
        ..Default::default()
    };
    // 0.1 amplitude ≈ -20 dBFS, well below threshold.
    let mut left = vec![0.1_f32; 4096];
    let mut right = left.clone();
    let expected = left.clone();
    c.process_stereo(&mut left, &mut right, &cfg);
    for (a, b) in left.iter().zip(expected.iter()) {
        assert!((a - b).abs() < 1e-6);
    }
}

#[test]
fn loud_signal_attenuated_by_expected_amount() {
    let mut c = GlueCompressor::new(48_000.0);
    let cfg = GlueCompressorConfig {
        enabled: true,
        threshold_db: -20.0,
        ratio: 8.0,
        attack_ms: 1.0,
        release_ms: 50.0,
        knee_db: 0.0,
        makeup_db: 0.0,
        mix: 1.0,
    };
    // 0.8 ≈ -1.94 dBFS → 18 dB over threshold, slope = 7/8 = 0.875,
    // so GR ≈ 15.75 dB in steady state.
    let frames = 4096;
    let mut left = vec![0.0_f32; frames];
    let mut right = vec![0.0_f32; frames];
    for i in 0..frames {
        let s = (i as f32 * 0.1).sin() * 0.8;
        left[i] = s;
        right[i] = s;
    }
    c.process_stereo(&mut left, &mut right, &cfg);
    // Measure settled-tail peak.
    let tail = &left[frames * 3 / 4..];
    let peak = tail.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    // Settled peak should be well below the 0.8 input peak.
    assert!(peak < 0.25, "settled peak = {peak}");
    // GR meter should be reporting something substantial.
    assert!(c.meter_gr_db() > 10.0, "gr = {}", c.meter_gr_db());
}

#[test]
fn anti_phase_wide_signal_is_compressed() {
    // A loud side-only signal: L = sine, R = −sine. The old mono-sum
    // detector read this as silence and applied no gain reduction at
    // all; the max-of-channels detector must see the full level.
    let mut c = GlueCompressor::new(48_000.0);
    let cfg = GlueCompressorConfig {
        enabled: true,
        threshold_db: -20.0,
        ratio: 8.0,
        attack_ms: 1.0,
        release_ms: 50.0,
        knee_db: 0.0,
        makeup_db: 0.0,
        mix: 1.0,
    };
    // 0.8 ≈ −1.94 dBFS per channel → 18 dB over threshold, slope 7/8,
    // so ~15.75 dB of GR in steady state — same as the centered case.
    let frames = 4096;
    let mut left = vec![0.0_f32; frames];
    let mut right = vec![0.0_f32; frames];
    for i in 0..frames {
        let s = (i as f32 * 0.1).sin() * 0.8;
        left[i] = s;
        right[i] = -s;
    }
    c.process_stereo(&mut left, &mut right, &cfg);
    let tail = &left[frames * 3 / 4..];
    let peak = tail.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    assert!(peak < 0.25, "settled anti-phase peak = {peak}");
    assert!(c.meter_gr_db() > 10.0, "gr = {}", c.meter_gr_db());
}

#[test]
fn hard_panned_detects_like_centered() {
    // The same per-channel level, hard-panned left versus centered,
    // must be detected within 0.5 dB of each other. The old mono-sum
    // detector read the panned signal 6 dB low.
    let cfg = GlueCompressorConfig {
        enabled: true,
        threshold_db: -20.0,
        ratio: 8.0,
        attack_ms: 1.0,
        release_ms: 50.0,
        knee_db: 0.0,
        makeup_db: 0.0,
        mix: 1.0,
    };
    let frames = 4096;
    let signal: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.1).sin() * 0.8).collect();

    let mut panned = GlueCompressor::new(48_000.0);
    let mut pl = signal.clone();
    let mut pr = vec![0.0_f32; frames];
    panned.process_stereo(&mut pl, &mut pr, &cfg);

    let mut centered = GlueCompressor::new(48_000.0);
    let mut cl = signal.clone();
    let mut cr = signal.clone();
    centered.process_stereo(&mut cl, &mut cr, &cfg);

    let diff = (panned.meter_gr_db() - centered.meter_gr_db()).abs();
    assert!(
        diff < 0.5,
        "hard-panned GR {} dB vs centered GR {} dB",
        panned.meter_gr_db(),
        centered.meter_gr_db()
    );
}

#[test]
fn disable_under_gain_reduction_releases_without_click() {
    // With ~15 dB of GR held on a 60 Hz sine, toggling the stage off
    // used to zero `gr_db` instantly — a full-GR level step in one
    // sample. It must now relax to unity at the release rate.
    let sr = 48_000.0_f32;
    let cfg_on = GlueCompressorConfig {
        enabled: true,
        threshold_db: -20.0,
        ratio: 8.0,
        attack_ms: 1.0,
        release_ms: 50.0,
        knee_db: 0.0,
        makeup_db: 0.0,
        mix: 1.0,
    };
    let cfg_off = GlueCompressorConfig {
        enabled: false,
        ..cfg_on
    };

    // 60 Hz → 800-sample period; 100-frame blocks put the toggle
    // boundary at sample 9800, a quarter period past a cycle start —
    // i.e. exactly on a peak, the worst case for a level step.
    let block = 100;
    let toggle = 9800;
    let total = 14_000;
    let input: Vec<f32> = (0..total)
        .map(|i| (i as f32 / sr * 60.0 * std::f32::consts::TAU).sin() * 0.8)
        .collect();

    let render = |toggle_at: Option<usize>| -> Vec<f32> {
        let mut c = GlueCompressor::new(sr);
        let mut l = input.clone();
        let mut r = input.clone();
        let mut start = 0;
        while start < total {
            let end = (start + block).min(total);
            let cfg = match toggle_at {
                Some(t) if start >= t => &cfg_off,
                _ => &cfg_on,
            };
            let (lh, rh) = (&mut l[start..end], &mut r[start..end]);
            c.process_stereo(lh, rh, cfg);
            start = end;
        }
        l
    };

    let steady = render(None);
    let toggled = render(Some(toggle));

    let max_delta = |x: &[f32]| {
        x[toggle - 400..toggle + 2400]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0_f32, f32::max)
    };
    let steady_max = max_delta(&steady);
    let toggled_max = max_delta(&toggled);
    // The old hard toggle stepped ~0.67 here; the release fade keeps
    // the toggled run's slope in the same class as the steady one.
    assert!(
        toggled_max < steady_max.max(0.0063) * 2.0 + 0.01,
        "toggle stepped {toggled_max} per sample vs {steady_max} steady"
    );
}
