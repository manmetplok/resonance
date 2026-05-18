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
