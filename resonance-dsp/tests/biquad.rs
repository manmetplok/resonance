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

#[test]
fn assign_raw_sets_coefficients_and_preserves_state() {
    let mut b = Biquad::identity();
    b.assign_raw(0.5, 0.25, -0.25, -0.1, 0.05);
    assert_eq!(b.b0, 0.5);
    assert_eq!(b.b1, 0.25);
    assert_eq!(b.b2, -0.25);
    assert_eq!(b.a1, -0.1);
    assert_eq!(b.a2, 0.05);

    // Re-assigning the same coefficients mid-stream must not disturb the
    // delay line: the output sequence must match an uninterrupted run.
    let mut uninterrupted = Biquad::identity();
    uninterrupted.assign_raw(0.5, 0.25, -0.25, -0.1, 0.05);
    let mut reassigned = uninterrupted;
    let mut expect = Vec::new();
    let mut got = Vec::new();
    for (i, x) in [1.0f32, -0.5, 0.25, 0.75, -1.0].into_iter().enumerate() {
        expect.push(uninterrupted.process(x));
        if i == 2 {
            reassigned.assign_raw(0.5, 0.25, -0.25, -0.1, 0.05);
        }
        got.push(reassigned.process(x));
    }
    assert_eq!(expect, got);
}

#[test]
fn first_order_low_pass_is_minus_3_db_at_cutoff_and_6_db_per_octave() {
    let mut b = Biquad::identity();
    b.set_first_order_low_pass(SR, 1_000.0);
    assert!(db(b.magnitude(10.0, SR)).abs() < 0.01);
    assert!((db(b.magnitude(1_000.0, SR)) + 3.01).abs() < 0.05);
    // Two octaves up the asymptote is ~-12 dB (bilinear makes it a hair
    // steeper, never shallower).
    let at_4k = db(b.magnitude(4_000.0, SR));
    assert!(at_4k < -12.0 && at_4k > -13.5, "got {at_4k} dB");
}

#[test]
fn first_order_high_pass_is_minus_3_db_at_cutoff() {
    let mut b = Biquad::identity();
    b.set_first_order_high_pass(SR, 500.0);
    assert!((db(b.magnitude(500.0, SR)) + 3.01).abs() < 0.05);
    assert!(db(b.magnitude(20_000.0, SR)).abs() < 0.05);
    assert!(db(b.magnitude(62.5, SR)) < -17.5);
}

#[test]
fn first_order_analog_section_with_a_pole_above_nyquist_is_stable() {
    // A +12 dB high shelf with its geometric centre at 40 kHz: the pole
    // (80 kHz) is far above the 24 kHz Nyquist. It must still land inside
    // the unit circle and filter noise to a finite, bounded output.
    let g = 10f32.powf(12.0 / 20.0);
    let wc = 2.0 * std::f32::consts::PI * 40_000.0;
    let (wz, wp) = (wc / g.sqrt(), wc * g.sqrt());
    for sr in [44_100.0f32, 48_000.0] {
        let mut b = Biquad::identity();
        // H(s) = G (s + wz) / (s + wp) has unity DC gain and G at HF.
        b.set_first_order_analog(sr, g, g * wz, 1.0, wp, sr * 0.25);
        assert!(b.a1.abs() < 1.0, "pole outside the unit circle at {sr}");
        let mut peak = 0.0f32;
        let mut s = 1u32;
        for _ in 0..48_000 {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = (s >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0;
            let y = b.process(x);
            assert!(y.is_finite());
            peak = peak.max(y.abs());
        }
        assert!(peak < 8.0, "output ran away: {peak}");
    }
}
