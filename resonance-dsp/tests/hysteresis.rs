//! Jiles-Atherton hysteresis: the loop the primitive draws matches the
//! equations integrated offline at 16x the step, every solver stays
//! finite and bounded at any drive on full-scale noise, and the
//! Langevin series joins the closed form without a seam.

mod common;

use common::*;
use resonance_dsp::hysteresis::{
    langevin, langevin_deriv, langevin_pair, C_MAX, C_MIN, K, M_S_MAX, M_S_MIN,
};
use resonance_dsp::{Hysteresis, HysteresisSolver, JaParams, SimpleRng};

// ---------------------------------------------------------------------------
// Reference model
// ---------------------------------------------------------------------------

/// Chowdhury DAFx-19 eq. 18, written out here from the paper rather than
/// shared with the primitive, so the comparison checks the primitive's
/// maths as well as its integration: `dM/dt` for field `h` moving at
/// `hdot`.
fn reference_dm_dt(p: &JaParams, m: f64, h: f64, hdot: f64) -> f64 {
    let q = (h + p.alpha * m) / p.a;
    let (l, dl) = if q.abs() < 1e-4 {
        (q / 3.0, 1.0 / 3.0)
    } else {
        (1.0 / q.tanh() - 1.0 / q, 1.0 / (q * q) - 1.0 / (q.sinh() * q.sinh()))
    };
    let man = p.m_s * l;
    let delta = if hdot >= 0.0 { 1.0 } else { -1.0 };
    let delta_m = if (man - m) * delta > 0.0 { 1.0 } else { 0.0 };
    let num = (1.0 - p.c) * delta_m * (man - m) / ((1.0 - p.c) * delta * p.k - p.alpha * (man - m))
        * hdot
        + p.c * p.m_s / p.a * hdot * dl;
    num / (1.0 - p.c * p.alpha * p.m_s / p.a * dl)
}

/// The paper's time-domain RK4 (eq. 22) with `H` linearly interpolated
/// inside each step, run at `sub` sub-steps per input sample.
fn reference_run(p: &JaParams, h: &[f64], sr: f64, sub: usize) -> Vec<f64> {
    let t = 1.0 / (sr * sub as f64);
    let mut m = 0.0;
    let mut h_prev = 0.0;
    let mut out = Vec::with_capacity(h.len());
    for &hn in h {
        let hdot = (hn - h_prev) * sr;
        for s in 0..sub {
            let h0 = h_prev + (hn - h_prev) * s as f64 / sub as f64;
            let hm = h0 + 0.5 * hdot * t;
            let h1 = h0 + hdot * t;
            let k1 = t * reference_dm_dt(p, m, h0, hdot);
            let k2 = t * reference_dm_dt(p, m + 0.5 * k1, hm, hdot);
            let k3 = t * reference_dm_dt(p, m + 0.5 * k2, hm, hdot);
            let k4 = t * reference_dm_dt(p, m + k3, h1, hdot);
            m += (k1 + 2.0 * k2 + 2.0 * k3 + k4) / 6.0;
        }
        h_prev = hn;
        out.push(m);
    }
    out
}

fn primitive_run(p: JaParams, solver: HysteresisSolver, h: &[f64]) -> Vec<f64> {
    let mut hy = Hysteresis::new(p, solver);
    h.iter().map(|&x| hy.process(x)).collect()
}

/// Coercivity, remanence and peak magnetisation of the last full cycle
/// of a loop driven by a sine that starts at phase 0.
#[derive(Debug, Clone, Copy)]
struct Loop {
    coercivity: f64,
    remanence: f64,
    m_peak: f64,
}

fn measure_loop(h: &[f64], m: &[f64], period: usize) -> Loop {
    let start = h.len() - period;
    let mut hc = Vec::new();
    let mut mr = Vec::new();
    for i in start..h.len() - 1 {
        // M through zero: interpolate H there.
        if m[i] == 0.0 || m[i].signum() != m[i + 1].signum() {
            let f = m[i] / (m[i] - m[i + 1]);
            hc.push((h[i] + f * (h[i + 1] - h[i])).abs());
        }
        // H through zero: interpolate M there.
        if h[i] == 0.0 || h[i].signum() != h[i + 1].signum() {
            let f = h[i] / (h[i] - h[i + 1]);
            mr.push((m[i] + f * (m[i + 1] - m[i])).abs());
        }
    }
    assert!(hc.len() >= 2 && mr.len() >= 2, "not a loop: {hc:?} {mr:?}");
    let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    Loop {
        coercivity: avg(&hc),
        remanence: avg(&mr),
        m_peak: m[start..].iter().fold(0.0f64, |a, v| a.max(v.abs())),
    }
}

/// A slow sine: 50 Hz at 48 kHz, 4 cycles.
fn slow_sine(amp: f64) -> (Vec<f64>, usize) {
    let period = 960;
    (sine(50.0, amp, SR, 4 * period), period)
}

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1e-12)
}

/// Settings for the loop comparisons: a default loop, a hot, wide one,
/// and a clean, narrow one near saturation.
fn loop_cases() -> Vec<(&'static str, JaParams, f64)> {
    vec![
        ("default", JaParams::from_controls(1.0, 0.5, 0.5), 0.5),
        ("hot_underbiased", JaParams::from_controls(8.0, 0.8, 0.0), 1.0),
        ("clean_overbiased", JaParams::from_controls(2.0, 0.2, 1.0), 0.8),
    ]
}

#[test]
fn loop_shape_matches_the_reference_model() {
    for (name, p, amp) in loop_cases() {
        let (h, period) = slow_sine(amp);
        let reference = measure_loop(&h, &reference_run(&p, &h, SR, 16), period);
        eprintln!("{name:18} reference {reference:?}");
        // The loop is a real loop at these settings: a coercivity of the
        // fraction of k and a remanence clear of zero.
        assert!(reference.coercivity > 0.02 * K, "{name}: no coercivity");
        assert!(reference.remanence > 0.002 * p.m_s, "{name}: no remanence");
        for (solver, tol) in [
            (HysteresisSolver::Rk4, 0.005),
            (HysteresisSolver::Rk2, 0.02),
            (HysteresisSolver::NewtonRaphson, 0.02),
        ] {
            let got = measure_loop(&h, &primitive_run(p, solver, &h), period);
            eprintln!("{name:18} {solver:?} {got:?}");
            for (what, g, r) in [
                ("coercivity", got.coercivity, reference.coercivity),
                ("remanence", got.remanence, reference.remanence),
                ("peak M", got.m_peak, reference.m_peak),
            ] {
                assert!(
                    rel(g, r) < tol,
                    "{name}, {solver:?}: {what} {g:.6} vs reference {r:.6} (tolerance {tol})"
                );
            }
        }
    }
}

/// The whole curve, not just three points: the RK4 output tracks the
/// 16x reference sample by sample over the last cycle.
#[test]
fn rk4_tracks_the_reference_sample_by_sample() {
    for (name, p, amp) in loop_cases() {
        let (h, period) = slow_sine(amp);
        let reference = reference_run(&p, &h, SR, 16);
        let got = primitive_run(p, HysteresisSolver::Rk4, &h);
        let n = h.len();
        let err = (n - period..n).fold(0.0f64, |e, i| e.max((got[i] - reference[i]).abs()));
        assert!(err < 2e-3 * p.m_s, "{name}: max |ΔM| {err:.3e}");
    }
}

/// The steady-state loop closes: the last two cycles are the same loop.
#[test]
fn the_loop_is_periodic_in_steady_state() {
    let (h, period) = slow_sine(0.5);
    let m = primitive_run(JaParams::default(), HysteresisSolver::Rk4, &h);
    let n = h.len();
    for i in n - period..n {
        assert!((m[i] - m[i - period]).abs() < 1e-3, "sample {i}: {} vs {}", m[i], m[i - period]);
    }
}

/// A wider loop at lower bias: the `bias` → `c` mapping does what the
/// docs say.
#[test]
fn bias_narrows_the_loop() {
    let (h, period) = slow_sine(0.5);
    let mut last = f64::INFINITY;
    for bias in [0.0, 0.5, 1.0] {
        let p = JaParams::from_controls(1.0, 0.5, bias);
        let l = measure_loop(&h, &primitive_run(p, HysteresisSolver::Rk4, &h), period);
        assert!(l.coercivity < last, "bias {bias}: coercivity {} not below {last}", l.coercivity);
        last = l.coercivity;
    }
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

#[test]
fn controls_map_to_the_documented_constants() {
    let p = JaParams::from_controls(2.0, 0.0, 0.0);
    assert_eq!(p.m_s, M_S_MAX);
    assert_eq!(p.c, C_MIN);
    assert!((p.a - M_S_MAX / 6.0).abs() < 1e-15);
    assert_eq!(p.k, K);
    let p = JaParams::from_controls(2.0, 1.0, 1.0);
    assert_eq!(p.m_s, M_S_MIN);
    assert_eq!(p.c, C_MAX);
    // The anhysteretic slope is the drive gain, up to the mean field.
    for g in [0.25, 1.0, 15.85] {
        let s = JaParams::from_controls(g, 0.5, 0.5).anhysteretic_slope();
        assert!(rel(s, g) < 3e-3, "gain {g}: slope {s}");
    }
    // Garbage in, sane constants out.
    let p = JaParams::from_controls(f64::NAN, f64::NAN, f64::INFINITY);
    assert!(p.a.is_finite() && p.m_s.is_finite() && p.c.is_finite());
}

/// A quiet signal on the demagnetised material sees the initial
/// susceptibility `c·χ`; a loud one sees roughly the anhysteretic slope.
#[test]
fn quiet_signals_see_the_initial_susceptibility() {
    let p = JaParams::from_controls(1.0, 0.5, 0.5);
    let h = sine(1_000.0, 1e-4, SR, 480);
    let m = primitive_run(p, HysteresisSolver::Rk4, &h);
    let gain = m.iter().fold(0.0f64, |a, v| a.max(v.abs())) / 1e-4;
    assert!(rel(gain, p.initial_slope()) < 0.02, "gain {gain} vs {}", p.initial_slope());
}

// ---------------------------------------------------------------------------
// Stability
// ---------------------------------------------------------------------------

fn noise(n: usize, amp: f64, seed: u64) -> Vec<f64> {
    let mut rng = SimpleRng::new(seed);
    (0..n)
        .map(|_| amp * (2.0 * ((rng.next_u32() >> 8) as f64 / (1u32 << 24) as f64) - 1.0))
        .collect()
}

#[test]
fn no_nan_or_blow_up_at_max_drive_on_full_scale_noise() {
    for solver in HysteresisSolver::ALL {
        for gain in [1.0, 15.85, 1_000.0] {
            for (sat, bias) in [(0.0, 0.0), (1.0, 1.0), (0.5, 0.0), (1.0, 0.0)] {
                for amp in [1.0, 4.0, 1.0e4] {
                    let p = JaParams::from_controls(gain, sat, bias);
                    let h = noise(48_000, amp, 7);
                    let m = primitive_run(p, solver, &h);
                    let tag = format!("{solver:?} gain {gain} sat {sat} bias {bias} amp {amp}");
                    assert!(m.iter().all(|v| v.is_finite()), "{tag}: non-finite");
                    assert!(m.iter().all(|v| v.abs() <= p.m_s), "{tag}: |M| above M_s");
                    // Not stuck on a rail: the output still moves with the
                    // input.
                    let mean = m.iter().sum::<f64>() / m.len() as f64;
                    let var = m.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / m.len() as f64;
                    assert!(var.sqrt() > 0.1 * p.m_s, "{tag}: stuck (σ {:.3e})", var.sqrt());
                    assert!(mean.abs() < 0.2 * p.m_s, "{tag}: drifted to {mean}");
                }
            }
        }
    }
}

#[test]
fn non_finite_input_reads_as_zero_field() {
    let mut hy = Hysteresis::default();
    for x in [0.3, f64::NAN, f64::INFINITY, -f64::INFINITY, 0.1] {
        assert!(hy.process(x).is_finite());
    }
}

#[test]
fn reset_returns_to_the_demagnetised_state() {
    let p = JaParams::from_controls(3.0, 0.5, 0.3);
    let h = noise(2_000, 0.7, 3);
    let fresh = primitive_run(p, HysteresisSolver::Rk4, &h);
    let mut hy = Hysteresis::new(p, HysteresisSolver::Rk4);
    for &x in &noise(5_000, 1.0, 9) {
        hy.process(x);
    }
    hy.reset();
    assert_eq!(hy.magnetisation(), 0.0);
    let again: Vec<f64> = h.iter().map(|&x| hy.process(x)).collect();
    assert_eq!(again, fresh);
}

/// Symmetric model, symmetric loop: odd harmonics only.
#[test]
fn the_loop_makes_odd_harmonics_only() {
    let n = 4_800;
    let h = sine(1_000.0, 0.3, 96_000.0, 3 * n);
    let m = primitive_run(JaParams::from_controls(3.0, 0.5, 0.5), HysteresisSolver::Rk4, &h);
    let tail: Vec<f32> = m[2 * n..].iter().map(|&v| v as f32).collect();
    let spec = amplitude_spectrum(&tail);
    // 1 kHz at 96 kHz over 4800 samples: bin 50 per harmonic.
    let h1 = spec[50];
    let db = |k: usize| 20.0 * (spec[50 * k] / h1).log10();
    assert!(db(3) > -60.0, "H3 {:.1} dBc: the loop is not distorting", db(3));
    for k in [2, 4, 6] {
        assert!(db(k) < -100.0, "H{k} {:.1} dBc from a symmetric loop", db(k));
    }
}

// ---------------------------------------------------------------------------
// Langevin
// ---------------------------------------------------------------------------

#[test]
fn langevin_series_meets_the_closed_form() {
    // Closed form in f64 is accurate well away from 0; compare the
    // primitive on both sides of its series switch against it and
    // against the series.
    for x in [0.009_999f64, 0.010_001, 0.02, 0.1, 1.0, 5.0, 30.0, 800.0] {
        let exact = 1.0 / x.tanh() - 1.0 / x;
        let dexact = if x < 300.0 {
            1.0 / (x * x) - 1.0 / (x.sinh() * x.sinh())
        } else {
            1.0 / (x * x)
        };
        assert!(rel(langevin(x), exact) < 1e-9, "L({x}) {} vs {exact}", langevin(x));
        assert!(rel(langevin_deriv(x), dexact) < 1e-7, "L'({x}) {} vs {dexact}", langevin_deriv(x));
        // Odd and even.
        assert_eq!(langevin(-x), -langevin(x));
        assert_eq!(langevin_deriv(-x), langevin_deriv(x));
    }
    // Near zero: the series, exactly 1/3 slope at the origin, no NaN.
    assert_eq!(langevin(0.0), 0.0);
    assert_eq!(langevin_deriv(0.0), 1.0 / 3.0);
    for x in [1e-300, 1e-12, 1e-6, 1e-3] {
        let (l, dl) = langevin_pair(x);
        assert!(rel(l, x / 3.0 - x * x * x / 45.0) < 1e-12, "L({x})");
        assert!(rel(dl, 1.0 / 3.0 - x * x / 15.0) < 1e-12, "L'({x})");
    }
    // Saturation.
    assert!((langevin(1e6) - 1.0).abs() < 1e-5);
    assert!(langevin_deriv(1e6) >= 0.0);
}
