//! Probe analysis (warmth-width-depth.md §7.3, slice W3): harmonics, THD,
//! H2/H3, decay, aliasing floor and SMPTE IMD, against nonlinearities
//! whose spectra are known in closed form.

use resonance_metering::probe::{
    analyze_harmonics, bin_exact_hz, imd_pct, probe_sine, smpte_pair, FLOOR_DBC, PROBE_LEN,
};

const SR: f64 = 48_000.0;

fn db(x: f64) -> f64 {
    20.0 * x.log10()
}

/// A bin-exact probe at `freq`, `amp` peak, through `f`.
fn through(freq: f64, amp: f64, f: impl Fn(f64) -> f64) -> (f64, Vec<f32>) {
    let freq = bin_exact_hz(SR, freq);
    let x = probe_sine(SR, freq, db(amp), PROBE_LEN);
    (freq, x.iter().map(|&s| f(s as f64) as f32).collect())
}

/// Fourier sine coefficient `b_k` of `f(A·sin θ)` by dense numerical
/// integration over one period: the analytic answer for a memoryless
/// odd shaper, independent of the FFT path under test.
fn sine_coefficient(f: impl Fn(f64) -> f64, amp: f64, k: usize) -> f64 {
    let n = 200_000;
    let step = std::f64::consts::TAU / n as f64;
    (0..n)
        .map(|i| {
            let theta = (i as f64 + 0.5) * step;
            f(amp * theta.sin()) * (k as f64 * theta).sin()
        })
        .sum::<f64>()
        * step
        / std::f64::consts::PI
}

#[test]
fn a_clean_sine_has_no_harmonics_and_no_aliasing() {
    let (freq, y) = through(1_000.0, 0.5, |x| x);
    let r = analyze_harmonics(SR, freq, &y);
    assert!((r.freq_hz - freq).abs() < 1e-9);
    assert!((r.fundamental_dbfs - db(0.5)).abs() < 0.01, "{}", r.fundamental_dbfs);
    assert!(r.thd_pct < 1e-4, "{}", r.thd_pct);
    for level in r.h.iter().flatten() {
        assert!(*level < -120.0, "{:?}", r.h);
    }
    assert!(r.aliasing_floor_dbc < -120.0, "{}", r.aliasing_floor_dbc);
    assert_eq!(r.decay_db_per_order, None, "nothing to fit");
}

#[test]
fn a_square_law_term_gives_exactly_its_second_harmonic() {
    // y = x + c·x²: H2 = c·A²/2, fundamental A, no H3.
    let (a, c) = (0.5, 0.1);
    let (freq, y) = through(1_000.0, a, |x| x + c * x * x);
    let r = analyze_harmonics(SR, freq, &y);
    let want = db(c * a / 2.0);
    let h2 = r.h[0].unwrap();
    assert!((h2 - want).abs() < 0.01, "H2 {h2} dBc, analytic {want}");
    assert!(r.h[1].unwrap() < -120.0, "no H3 from a square law");
    assert!(r.h2_h3_db.unwrap() > 60.0, "even-dominant");
    let thd = 100.0 * c * a / 2.0;
    assert!((r.thd_pct - thd).abs() < 1e-3, "THD {} %, analytic {thd}", r.thd_pct);
}

#[test]
fn a_cubic_term_gives_exactly_its_third_harmonic() {
    // y = x − c·x³: fundamental A − 3cA³/4, H3 = cA³/4.
    let (a, c) = (0.8, 0.2);
    let (freq, y) = through(1_000.0, a, |x| x - c * x * x * x);
    let r = analyze_harmonics(SR, freq, &y);
    let fundamental = a - 0.75 * c * a * a * a;
    let want = db(0.25 * c * a * a * a / fundamental);
    let h3 = r.h[1].unwrap();
    assert!((h3 - want).abs() < 0.01, "H3 {h3} dBc, analytic {want}");
    assert!((r.fundamental_dbfs - db(fundamental)).abs() < 0.01);
    assert!(r.h[0].unwrap() < -120.0, "no H2 from an odd shaper");
    assert!(r.h2_h3_db.unwrap() < -60.0, "odd-dominant");
}

#[test]
fn tanh_at_known_drive_matches_its_analytic_harmonics() {
    for drive in [1.0f64, 2.0, 4.0] {
        let shaper = move |x: f64| (drive * x).tanh();
        let amp = 0.5;
        let (freq, y) = through(1_000.0, amp, shaper);
        let r = analyze_harmonics(SR, freq, &y);
        let b1 = sine_coefficient(shaper, amp, 1);
        for order in [3usize, 5, 7, 9] {
            let want = db(sine_coefficient(shaper, amp, order).abs() / b1);
            let got = r.h[order - 2].unwrap();
            if want > -130.0 {
                assert!(
                    (got - want).abs() < 0.05,
                    "drive {drive}: H{order} {got} dBc, analytic {want}"
                );
            }
        }
        for even in [2usize, 4, 6, 8] {
            assert!(r.h[even - 2].unwrap() < -120.0, "tanh is odd: H{even} {:?}", r.h);
        }
        assert!(r.decay_db_per_order.unwrap() > 0.0, "the series falls");
        assert!(r.h2_h3_db.unwrap() < 0.0);
    }
}

#[test]
fn harmonics_above_nyquist_are_none_and_fold_into_the_aliasing_floor() {
    // A hard clipper at 5 kHz: H5 (25 kHz) and up are past Nyquist.
    let (freq, y) = through(5_000.0, 1.0, |x| x.clamp(-0.5, 0.5));
    let r = analyze_harmonics(SR, freq, &y);
    assert!(r.h[3].is_none() && r.h[7].is_none(), "{:?}", r.h);
    assert!(r.h[1].is_some(), "H3 = 15 kHz is in band");
    assert!(r.aliasing_floor_dbc > -40.0, "folded harmonics show: {}", r.aliasing_floor_dbc);
    // The same clipper at 1 kHz aliases far less: its fold-backs start at
    // H25.
    let (freq, y) = through(1_000.0, 1.0, |x| x.clamp(-0.5, 0.5));
    let low = analyze_harmonics(SR, freq, &y);
    assert!(low.aliasing_floor_dbc < r.aliasing_floor_dbc - 10.0);
}

#[test]
fn silence_out_reports_the_floor_not_nan() {
    let (freq, _) = through(1_000.0, 0.5, |x| x);
    let r = analyze_harmonics(SR, freq, &vec![0.0f32; PROBE_LEN]);
    assert_eq!(r.thd_pct, 0.0);
    assert!(r.h.iter().flatten().all(|&l| l == FLOOR_DBC));
    assert_eq!(r.aliasing_floor_dbc, FLOOR_DBC);
}

#[test]
fn smpte_imd_matches_the_square_law_sidebands() {
    let (x, low, high) = smpte_pair(SR, -6.0, PROBE_LEN);
    assert!((low - 60.0).abs() < 0.5 && (high - 7_000.0).abs() < 0.5);
    let clean = imd_pct(SR, low, high, &x).unwrap();
    assert!(clean < 1e-4, "a linear chain has no IMD: {clean}");

    // x² puts c·a·b at high ± low: IMD = √2·c·a·100 with a the low
    // tone's amplitude.
    let c = 0.1;
    let a = 10f64.powf(-6.0 / 20.0) * 0.8;
    let y: Vec<f32> = x.iter().map(|&s| (s as f64 + c * (s as f64).powi(2)) as f32).collect();
    let got = imd_pct(SR, low, high, &y).unwrap();
    let want = std::f64::consts::SQRT_2 * c * a * 100.0;
    assert!((got - want).abs() < 1e-3 * want.max(1.0), "IMD {got} %, analytic {want}");
}
