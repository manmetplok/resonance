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
