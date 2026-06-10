use resonance_dsp::DcBlocker;

/// A constant DC input decays toward zero output once the blocker settles.
#[test]
fn removes_static_dc_offset() {
    let mut b = DcBlocker::default();
    let mut y = 0.0;
    for _ in 0..8192 {
        y = b.process(0.5);
    }
    assert!(y.abs() < 1e-4, "residual DC = {y}");
}

/// A DC-offset sine comes out with near-zero mean while the AC component
/// passes essentially unchanged.
#[test]
fn strips_offset_but_passes_audio() {
    let sr = 48_000.0_f32;
    let f0 = 750.0_f32; // 64 samples per period
    let mut b = DcBlocker::default();
    let n = 8192;
    let mut out = vec![0.0_f32; n];
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f32 / sr;
        *o = b.process((std::f32::consts::TAU * f0 * t).sin() * 0.5 + 0.3);
    }

    // Measure over the trailing whole periods, after the ~200-sample
    // time constant has long passed.
    let tail = &out[n - 4096..];
    let mean = tail.iter().sum::<f32>() / tail.len() as f32;
    assert!(mean.abs() < 1e-4, "mean = {mean}");
    let peak = tail.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    assert!((peak - 0.5).abs() < 0.01, "peak = {peak}");
}

/// `reset` clears the filter state.
#[test]
fn reset_clears_state() {
    let mut b = DcBlocker::default();
    for _ in 0..100 {
        b.process(1.0);
    }
    b.reset();
    let mut fresh = DcBlocker::default();
    assert_eq!(b.process(0.25), fresh.process(0.25));
}
