use resonance_metering::CrestMeter;

#[test]
fn sine_wave_crest_is_about_three_db() {
    // A unit sine has peak = 1, RMS = 1/sqrt(2) → crest = 3.01 dB.
    let sr = 48_000.0;
    let mut m = CrestMeter::new(sr);
    let mut l = vec![0.0_f32; (sr * 0.2) as usize];
    for (i, s) in l.iter_mut().enumerate() {
        *s = (i as f32 / sr * 1000.0 * std::f32::consts::TAU).sin();
    }
    let r = l.clone();
    m.push_stereo(&l, &r);
    let crest = m.crest_db();
    assert!((crest - 3.01).abs() < 0.5, "crest = {crest}");
}

#[test]
fn silence_is_zero_crest() {
    let m = CrestMeter::new(48_000.0);
    assert_eq!(m.crest_db(), 0.0);
}
