use resonance_metering::CorrelationMeter;

#[test]
fn identical_channels_correlate_to_plus_one() {
    let sr = 48_000.0;
    let mut m = CorrelationMeter::new(sr);
    let mut l = vec![0.0_f32; (sr * 0.2) as usize];
    for (i, s) in l.iter_mut().enumerate() {
        *s = ((i as f32) * 0.01).sin();
    }
    let r = l.clone();
    m.push_stereo(&l, &r);
    assert!(
        (m.correlation() - 1.0).abs() < 1e-3,
        "got {}",
        m.correlation()
    );
}

#[test]
fn inverted_channels_correlate_to_minus_one() {
    let sr = 48_000.0;
    let mut m = CorrelationMeter::new(sr);
    let mut l = vec![0.0_f32; (sr * 0.2) as usize];
    for (i, s) in l.iter_mut().enumerate() {
        *s = ((i as f32) * 0.01).sin();
    }
    let r: Vec<f32> = l.iter().map(|&x| -x).collect();
    m.push_stereo(&l, &r);
    assert!(
        (m.correlation() + 1.0).abs() < 1e-3,
        "got {}",
        m.correlation()
    );
}

#[test]
fn silence_reports_zero() {
    let m = CorrelationMeter::new(48_000.0);
    assert_eq!(m.correlation(), 0.0);
}

/// Until the ~100 ms window has filled, the readout stays gated at the
/// neutral 0.0 even for perfectly correlated input.
#[test]
fn readout_gated_until_window_full() {
    let sr = 48_000.0;
    let window = (sr * 0.1) as usize;
    let mut m = CorrelationMeter::new(sr);

    // Push one sample short of a full window of identical channels.
    let mut l = vec![0.0_f32; window - 1];
    for (i, s) in l.iter_mut().enumerate() {
        *s = ((i as f32) * 0.01).sin();
    }
    let r = l.clone();
    m.push_stereo(&l, &r);
    assert_eq!(
        m.correlation(),
        0.0,
        "partial window must read neutral 0.0"
    );

    // One more sample completes the window: readout opens at +1.
    m.push_stereo(&[0.5], &[0.5]);
    assert!(
        (m.correlation() - 1.0).abs() < 1e-3,
        "got {}",
        m.correlation()
    );
}

/// `reset` re-arms the gate.
#[test]
fn reset_rearms_the_gate() {
    let sr = 48_000.0;
    let window = (sr * 0.1) as usize;
    let mut m = CorrelationMeter::new(sr);

    let mut l = vec![0.0_f32; window];
    for (i, s) in l.iter_mut().enumerate() {
        *s = ((i as f32) * 0.01).sin();
    }
    let r = l.clone();
    m.push_stereo(&l, &r);
    assert!((m.correlation() - 1.0).abs() < 1e-3);

    m.reset();
    m.push_stereo(&l[..window / 2], &r[..window / 2]);
    assert_eq!(
        m.correlation(),
        0.0,
        "half-full window after reset must read neutral 0.0"
    );
}

/// DSP2-16: one NaN or Inf sample used to poison the running sums until
/// reset. Once it has left the window the meter must read normally again.
#[test]
fn non_finite_sample_does_not_stick() {
    let sr = 48_000.0;
    let mut m = CorrelationMeter::new(sr);
    let n = (sr * 0.2) as usize;
    let mut l: Vec<f32> = (0..n).map(|i| (i as f32 * 0.01).sin()).collect();
    l[10] = f32::NAN;
    let mut r = l.clone();
    r[20] = f32::INFINITY;
    m.push_stereo(&l, &r);
    let c = m.correlation();
    assert!(
        (c - 1.0).abs() < 1e-3,
        "after a NaN/Inf that left the window: {c}"
    );
}
