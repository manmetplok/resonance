use resonance_mastering::stages::multiband::lowpass::LinearPhaseLowpass;

#[test]
fn lowpass_attenuates_above_cutoff() {
    let sr = 48_000.0_f32;
    let mut lp = LinearPhaseLowpass::new(sr, 1000.0);
    let latency = LinearPhaseLowpass::latency();
    let n = latency + 4096;

    // 5 kHz sine — well above 1 kHz cutoff → should be much quieter.
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    for i in 0..n {
        let s = (i as f32 / sr * 5000.0 * std::f32::consts::TAU).sin() * 0.5;
        l[i] = s;
        r[i] = s;
    }
    lp.process_stereo(&mut l, &mut r);
    let tail = &l[latency + 2048..];
    let peak = tail.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    // 5 kHz is ~2.3 octaves above 1 kHz cutoff, 24 dB/oct → ~55 dB down
    // → amplitude below ~0.001.
    assert!(peak < 0.01, "5 kHz through 1 kHz LP: peak = {peak}");
}

#[test]
fn lowpass_passes_below_cutoff() {
    let sr = 48_000.0_f32;
    let mut lp = LinearPhaseLowpass::new(sr, 1000.0);
    let latency = LinearPhaseLowpass::latency();
    let n = latency + 4096;

    // 200 Hz sine — well below 1 kHz → should pass near-unity.
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    for i in 0..n {
        let s = (i as f32 / sr * 200.0 * std::f32::consts::TAU).sin() * 0.5;
        l[i] = s;
        r[i] = s;
    }
    lp.process_stereo(&mut l, &mut r);
    let tail = &l[latency + 2048..];
    let peak = tail.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    assert!(
        (peak - 0.5).abs() < 0.02,
        "200 Hz through 1 kHz LP: peak = {peak} (expected ≈ 0.5)"
    );
}
