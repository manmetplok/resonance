use resonance_dsp::Biquad;

const SR: f32 = 48_000.0;

fn db(linear: f32) -> f32 {
    20.0 * linear.max(1e-12).log10()
}

#[test]
fn identity_passes_signal_through() {
    let mut b = Biquad::identity();
    for x in [0.0f32, 0.5, -0.7, 1.0, -1.0] {
        assert!((b.process(x) - x).abs() < 1e-6);
    }
}

#[test]
fn bell_hits_target_gain_at_center() {
    let mut b = Biquad::identity();
    b.set_bell(SR, 1_000.0, 1.0, 6.0);
    let mag_db = db(b.magnitude(1_000.0, SR));
    assert!((mag_db - 6.0).abs() < 0.1, "got {mag_db} dB");

    b.set_bell(SR, 1_000.0, 1.0, -12.0);
    let mag_db = db(b.magnitude(1_000.0, SR));
    assert!((mag_db - (-12.0)).abs() < 0.1, "got {mag_db} dB");
}

#[test]
fn bell_is_flat_far_from_center() {
    let mut b = Biquad::identity();
    b.set_bell(SR, 1_000.0, 4.0, 12.0);
    // Two decades away the bell should be essentially flat.
    assert!(db(b.magnitude(10.0, SR)).abs() < 0.3);
    assert!(db(b.magnitude(20_000.0, SR)).abs() < 0.3);
}

#[test]
fn low_pass_is_unity_at_dc_and_attenuates_above_cutoff() {
    let mut b = Biquad::identity();
    b.set_low_pass(SR, 1_000.0, 0.707);
    assert!((db(b.magnitude(20.0, SR))).abs() < 0.1);
    // ~-3 dB at cutoff for Q=0.707.
    let at_cut = db(b.magnitude(1_000.0, SR));
    assert!((at_cut + 3.0).abs() < 0.5, "got {at_cut} dB at cutoff");
    // Well below unity one decade up.
    assert!(db(b.magnitude(10_000.0, SR)) < -30.0);
}

#[test]
fn high_pass_is_unity_well_above_cutoff() {
    let mut b = Biquad::identity();
    b.set_high_pass(SR, 200.0, 0.707);
    assert!((db(b.magnitude(20_000.0, SR))).abs() < 0.1);
    assert!(db(b.magnitude(20.0, SR)) < -30.0);
}

#[test]
fn low_shelf_reaches_target_gain_at_dc() {
    let mut b = Biquad::identity();
    b.set_low_shelf(SR, 200.0, 0.707, 6.0);
    let at_dc = db(b.magnitude(20.0, SR));
    assert!((at_dc - 6.0).abs() < 0.2, "got {at_dc} dB");
}

#[test]
fn high_shelf_reaches_target_gain_at_nyquist() {
    let mut b = Biquad::identity();
    b.set_high_shelf(SR, 8_000.0, 0.707, -6.0);
    let near_nyquist = db(b.magnitude(20_000.0, SR));
    assert!((near_nyquist - (-6.0)).abs() < 0.3, "got {near_nyquist} dB");
}

#[test]
fn cascaded_cuts_are_steeper() {
    let mut single = Biquad::identity();
    single.set_high_pass(SR, 200.0, 0.707);
    let s1 = db(single.magnitude(100.0, SR));

    let mut a = Biquad::identity();
    let mut b = Biquad::identity();
    a.set_high_pass(SR, 200.0, 0.707);
    b.set_high_pass(SR, 200.0, 0.707);
    let s2 = db(a.magnitude(100.0, SR)) + db(b.magnitude(100.0, SR));

    assert!(s2 < s1, "cascaded HP should attenuate more: {s1} vs {s2}");
}

#[test]
fn stable_at_extremes() {
    // High Q, near Nyquist, extreme gain — must produce finite coeffs.
    let mut b = Biquad::identity();
    b.set_bell(SR, 23_000.0, 10.0, 24.0);
    assert!(b.b0.is_finite() && b.a1.is_finite() && b.a2.is_finite());
    b.set_high_pass(SR, 5.0, 0.1);
    assert!(b.b0.is_finite() && b.a1.is_finite() && b.a2.is_finite());
}

#[test]
fn degenerate_sample_rate_does_not_panic() {
    // sr = 0 used to invert clamp_params' range (min 10 > max 0) and
    // panic inside f32::clamp. Degenerate rates must not panic in any
    // of the coefficient setters.
    let mut b = Biquad::identity();
    for sr in [0.0_f32, -48_000.0, f32::NAN] {
        b.set_bell(sr, 1_000.0, 1.0, 6.0);
        b.set_low_shelf(sr, 200.0, 0.707, 3.0);
        b.set_high_shelf(sr, 8_000.0, 0.707, -3.0);
        b.set_high_pass(sr, 100.0, 0.707);
        b.set_low_pass(sr, 10_000.0, 0.707);
    }
}
