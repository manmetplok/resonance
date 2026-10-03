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
fn band_pass_peaks_at_0_db_on_its_centre() {
    let mut b = Biquad::identity();
    b.set_band_pass(SR, 2_000.0, 2.0);
    assert!(db(b.magnitude(2_000.0, SR)).abs() < 0.01);
    assert!(db(b.magnitude(200.0, SR)) < -20.0);
    assert!(db(b.magnitude(20_000.0, SR)) < -20.0);
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

// ---- DSP2-07: magnitude accuracy at low frequencies / high rates ----

/// Independent f64 reference: the RBJ prototype coefficients and a direct
/// complex evaluation of H(e^{jw}), both in f64.
fn reference_db(kind: &str, sr: f64, f0: f64, q: f64, g: f64, f: f64) -> f64 {
    use std::f64::consts::PI;
    let w0 = 2.0 * PI * f0 / sr;
    let (sn, cs) = w0.sin_cos();
    let al = sn / (2.0 * q);
    let a = 10f64.powf(g / 40.0);
    let sa = 2.0 * a.sqrt() * al;
    let (b0, b1, b2, a0, a1, a2) = match kind {
        "hp" => ((1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0, 1.0 + al, -2.0 * cs, 1.0 - al),
        "lp" => ((1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0, 1.0 + al, -2.0 * cs, 1.0 - al),
        "bell" => (1.0 + al * a, -2.0 * cs, 1.0 - al * a, 1.0 + al / a, -2.0 * cs, 1.0 - al / a),
        "ls" => (
            a * ((a + 1.0) - (a - 1.0) * cs + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
            a * ((a + 1.0) - (a - 1.0) * cs - sa),
            (a + 1.0) + (a - 1.0) * cs + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * cs),
            (a + 1.0) + (a - 1.0) * cs - sa,
        ),
        "hs" => (
            a * ((a + 1.0) + (a - 1.0) * cs + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
            a * ((a + 1.0) + (a - 1.0) * cs - sa),
            (a + 1.0) - (a - 1.0) * cs + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * cs),
            (a + 1.0) - (a - 1.0) * cs - sa,
        ),
        _ => unreachable!(),
    };
    let w = 2.0 * PI * f / sr;
    let (s1, c1) = w.sin_cos();
    let (s2, c2) = (2.0 * w).sin_cos();
    let nr = b0 + b1 * c1 + b2 * c2;
    let ni = -b1 * s1 - b2 * s2;
    let dr = a0 + a1 * c1 + a2 * c2;
    let di = -a1 * s1 - a2 * s2;
    10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).log10()
}

fn coeffs(kind: &str, sr: f64, f0: f64, q: f64, g: f64) -> resonance_dsp::BiquadCoeffs {
    use resonance_dsp::BiquadCoeffs as C;
    match kind {
        "hp" => C::high_pass(sr, f0, q),
        "lp" => C::low_pass(sr, f0, q),
        "bell" => C::bell(sr, f0, q, g),
        "ls" => C::low_shelf(sr, f0, q, g),
        "hs" => C::high_shelf(sr, f0, q, g),
        _ => unreachable!(),
    }
}

const CASES: &[(&str, f64, f64, f64)] = &[
    ("hp", 20.0, 0.707, 0.0),
    ("hp", 1_000.0, 0.707, 0.0),
    ("lp", 20.0, 0.707, 0.0),
    ("lp", 18_000.0, 0.707, 0.0),
    ("bell", 30.0, 2.0, 12.0),
    ("bell", 3_000.0, 1.0, -9.0),
    ("ls", 25.0, 0.707, 6.0),
    ("hs", 10_000.0, 0.707, -6.0),
];

/// Log sweep from 5 Hz to just under Nyquist.
fn sweep(sr: f64) -> impl Iterator<Item = f64> {
    let (lo, hi) = (5.0f64, sr * 0.499);
    (0..=400).map(move |i| lo * (hi / lo).powf(i as f64 / 400.0))
}

#[test]
fn f64_design_magnitude_matches_reference_from_5hz_to_nyquist() {
    for &sr in &[48_000.0, 96_000.0, 192_000.0] {
        for &(kind, f0, q, g) in CASES {
            let c = coeffs(kind, sr, f0, q, g);
            for f in sweep(sr) {
                let want = reference_db(kind, sr, f0, q, g, f);
                let got = 20.0 * c.magnitude(f, sr).log10();
                assert!(
                    (got - want).abs() < 0.05,
                    "{kind} {f0} Hz @ {sr}: {f:.1} Hz got {got:.3} dB, want {want:.3} dB"
                );
            }
        }
    }
}

/// `Biquad::magnitude` must be the true response of the f32 filter: the
/// same f32 coefficients evaluated in f64. The old f32 evaluation was off
/// by 1.6 dB at 5 Hz for a 20 Hz high-pass at 48 kHz, and by more at
/// higher rates.
#[test]
fn f32_filter_magnitude_has_no_evaluation_cancellation() {
    use std::f64::consts::PI;
    for &sr in &[48_000.0f32, 96_000.0, 192_000.0] {
        let mut b = Biquad::identity();
        b.set_high_pass(sr, 20.0, 0.707);
        let (b0, b1, b2) = (b.b0 as f64, b.b1 as f64, b.b2 as f64);
        let (a1, a2) = (b.a1 as f64, b.a2 as f64);
        for f in sweep(sr as f64) {
            let w = 2.0 * PI * f / sr as f64;
            let (s1, c1) = w.sin_cos();
            let (s2, c2) = (2.0 * w).sin_cos();
            let nr = b0 + b1 * c1 + b2 * c2;
            let ni = -b1 * s1 - b2 * s2;
            let dr = 1.0 + a1 * c1 + a2 * c2;
            let di = -a1 * s1 - a2 * s2;
            let want = 10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).log10();
            let got = 20.0 * (b.magnitude(f as f32, sr) as f64).log10();
            assert!(
                (got - want).abs() < 0.01,
                "hp 20 Hz @ {sr}: {f:.1} Hz got {got:.3} dB, want {want:.3} dB"
            );
        }
    }
}
