use resonance_metering::PlrMeter;

#[test]
fn plr_is_tp_minus_lufs() {
    let r = PlrMeter::compute(-1.0, -1.0, -14.0, -14.0);
    assert!((r.plr_db - 13.0).abs() < 1e-6);
    assert!((r.psr_db - 13.0).abs() < 1e-6);
}

#[test]
fn silent_input_yields_zero() {
    let r = PlrMeter::compute(
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    assert_eq!(r.plr_db, 0.0);
    assert_eq!(r.psr_db, 0.0);
}
