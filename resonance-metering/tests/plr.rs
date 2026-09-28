use resonance_metering::PlrMeter;

#[test]
fn plr_is_tp_minus_lufs() {
    let r = PlrMeter::compute(-1.0, -1.0, -14.0, -14.0);
    assert!((r.plr_db - 13.0).abs() < 1e-6);
    assert!((r.psr_db - 13.0).abs() < 1e-6);
}

#[test]
fn range_dynamics_use_the_loudest_short_term_window() {
    let r = PlrMeter::range(-1.0, -16.0, -12.0);
    assert_eq!(r.plr_db, Some(15.0), "true peak over integrated");
    assert_eq!(r.psr_db, Some(11.0), "true peak over the loudest 3 s window");
}

#[test]
fn range_dynamics_are_absent_not_zero_when_loudness_is_undefined() {
    // A 2 s range: integrated exists, the 3 s short-term window does not.
    let r = PlrMeter::range(-3.0, -20.0, f32::NEG_INFINITY);
    assert_eq!(r.plr_db, Some(17.0));
    assert_eq!(r.psr_db, None);
    let silent = PlrMeter::range(-120.0, f32::NEG_INFINITY, f32::NEG_INFINITY);
    assert_eq!((silent.plr_db, silent.psr_db), (None, None));
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
