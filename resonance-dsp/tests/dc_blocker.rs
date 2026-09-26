use resonance_dsp::DcBlocker;

/// A constant DC input decays toward zero output once the blocker settles.
#[test]
fn removes_static_dc_offset() {
    let mut b = DcBlocker::default();
    let mut y = 0.0;
    // 5 Hz corner at 48 kHz: τ ≈ 1.5k samples, so a second is ~31 τ.
    for _ in 0..48_000 {
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
    let n = 96_000;
    let mut out = vec![0.0_f32; n];
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f32 / sr;
        *o = b.process((std::f32::consts::TAU * f0 * t).sin() * 0.5 + 0.3);
    }

    // Measure over the trailing whole periods, after the ~200-sample
    // time constant (~1.5k samples at 5 Hz) has long passed.
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

/// Magnitude of the blocker's steady-state response to a sine, in dB.
fn sine_gain_db(cutoff_hz: f32, sr: f32, freq: f32) -> f64 {
    let mut b = DcBlocker::new(cutoff_hz, sr);
    let n = (sr * 4.0) as usize;
    let settle = n / 2;
    let (mut e_in, mut e_out) = (0.0_f64, 0.0_f64);
    for i in 0..n {
        let x = (std::f64::consts::TAU * freq as f64 * i as f64 / sr as f64).sin() as f32;
        let y = b.process(x);
        if i >= settle {
            e_in += (x as f64).powi(2);
            e_out += (y as f64).powi(2);
        }
    }
    assert!(e_out > 0.0, "blocker output is silent");
    10.0 * (e_out / e_in).log10()
}

/// DSP-04: the corner is fixed in Hz, not in samples. At every project
/// rate a 40 Hz sine passes within 0.1 dB through the default (5 Hz)
/// blocker and the −3 dB point lands on the requested cutoff.
#[test]
fn corner_is_independent_of_sample_rate() {
    for sr in [44_100.0_f32, 48_000.0, 96_000.0, 192_000.0] {
        let g40 = sine_gain_db(DcBlocker::DEFAULT_CUTOFF_HZ, sr, 40.0);
        assert!(g40 > -0.1, "{sr} Hz: 40 Hz through the default blocker = {g40:.3} dB");
        let corner = sine_gain_db(20.0, sr, 20.0);
        assert!(
            (corner + 3.01).abs() < 0.1,
            "{sr} Hz: gain at a 20 Hz corner = {corner:.3} dB (want -3 dB)"
        );
        // DC still decays.
        let mut b = DcBlocker::new(DcBlocker::DEFAULT_CUTOFF_HZ, sr);
        let mut y = 0.0;
        for _ in 0..(sr as usize) {
            y = b.process(0.5);
        }
        assert!(y.abs() < 1e-4, "{sr} Hz: residual DC = {y}");
    }
}
