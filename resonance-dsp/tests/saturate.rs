//! The saturation curves and their first-order ADAA form: harmonic
//! signatures, agreement with the analytic `tanh` series, the aliasing
//! floor with the IIR oversampler, DC behaviour and extreme inputs.

mod common;

use common::*;
use resonance_dsp::saturate::{Adaa1, Curve};
use resonance_dsp::{OversampleFactor, Oversampler};

/// 0.2 s windows: 5 Hz bins, so every test tone (100 Hz, 1 kHz, 5 kHz)
/// and every alias of them (all multiples of 100 Hz) is coherent.
const WINDOW: usize = 9_600;
const BIN_HZ: f64 = SR / WINDOW as f64;
const SETTLE: usize = 2_400;

fn bin(freq: f64) -> usize {
    (freq / BIN_HZ).round() as usize
}

fn curves() -> Vec<(&'static str, Curve)> {
    vec![
        ("tanh", Curve::Tanh),
        ("tube", Curve::Tube { bias: 0.5 }),
        ("warm", Curve::Warm { amount: 1.0 }),
        ("console", Curve::Console { drive: 1.0 }),
        ("clip_soft", Curve::Clip { shape: 0.0 }),
        ("clip_mid", Curve::Clip { shape: 0.5 }),
        ("clip_hard", Curve::Clip { shape: 1.0 }),
        ("inflator", Curve::Inflator { curve: 0.0 }),
    ]
}

#[derive(Clone, Copy, Debug)]
enum Aa {
    Naive,
    Adaa,
}

/// Run a sine of peak `peak` (in the driven domain) at `freq` through
/// `curve` at oversampling `ratio` (1, 2, 4 or 8; 8 cascades a 2× outer
/// and a 4× inner `Oversampler`), returning the measurement window.
fn render(curve: Curve, aa: Aa, ratio: usize, freq: f64, peak: f64) -> Vec<f32> {
    let x = sine(freq, peak, SR, SETTLE + WINDOW);
    let mut adaa = Adaa1::new();
    let mut shape = |u: f32| -> f32 {
        match aa {
            Aa::Naive => curve.eval(u as f64) as f32,
            Aa::Adaa => adaa.process(&curve, u),
        }
    };
    let mut outer = Oversampler::new();
    let mut inner = Oversampler::new();
    let out: Vec<f32> = match ratio {
        1 => x.iter().map(|&s| shape(s as f32)).collect(),
        2 | 4 => {
            outer.set_factor(if ratio == 2 { OversampleFactor::X2 } else { OversampleFactor::X4 });
            x.iter()
                .map(|&s| {
                    let mut buf = outer.upsample(s as f32);
                    for v in &mut buf[..ratio] {
                        *v = shape(*v);
                    }
                    outer.downsample(&buf)
                })
                .collect()
        }
        8 => {
            outer.set_factor(OversampleFactor::X2);
            inner.set_factor(OversampleFactor::X4);
            x.iter()
                .map(|&s| {
                    let mut mid = outer.upsample(s as f32);
                    for m in &mut mid[..2] {
                        let mut buf = inner.upsample(*m);
                        for v in &mut buf {
                            *v = shape(*v);
                        }
                        *m = inner.downsample(&buf);
                    }
                    outer.downsample(&mid)
                })
                .collect()
        }
        _ => unreachable!(),
    };
    out[SETTLE..].to_vec()
}

/// Absent harmonics (odd ones of an even curve, even ones of an odd
/// curve) must sit below this.
const ABSENT_DBC: f64 = -140.0;

/// The harmonic signature of each curve: a 1 kHz sine at driven peak
/// `peak`, first-order ADAA inside the 4× oversampler (so neither aliasing
/// nor the ADAA's own lowpass touches the first nine harmonics). `None`
/// means "absent" (below [`ABSENT_DBC`]). Decay is the least-squares
/// slope over the present harmonics among H2..H9.
struct Signature {
    name: &'static str,
    curve: Curve,
    peak: f64,
    h2: Option<f64>,
    h3: Option<f64>,
    decay: f64,
}

fn signatures() -> Vec<Signature> {
    let s = |name, curve, peak, h2, h3, decay| Signature { name, curve, peak, h2, h3, decay };
    vec![
        // Low level (a −6 dBFS sine at unity drive): the warmth range.
        s("tanh", Curve::Tanh, 0.5, None, Some(-34.15), -16.13),
        s("tube", Curve::Tube { bias: 0.5 }, 0.5, Some(-19.51), Some(-41.74), -16.26),
        s("warm", Curve::Warm { amount: 1.0 }, 0.5, Some(-24.59), None, -16.18),
        s("console", Curve::Console { drive: 1.0 }, 0.5, None, Some(-39.51), -25.03),
        // Driven (a 0 dBFS sine at +6 dB).
        s("tanh", Curve::Tanh, 2.0, None, Some(-15.45), -6.33),
        s("tube", Curve::Tube { bias: 0.5 }, 2.0, Some(-14.95), Some(-17.67), -7.29),
        s("warm", Curve::Warm { amount: 1.0 }, 2.0, Some(-16.39), None, -7.86),
        s("console", Curve::Console { drive: 1.0 }, 2.0, None, Some(-14.40), -8.75),
        s("clip_soft", Curve::Clip { shape: 0.0 }, 2.0, None, Some(-16.63), -5.48),
        s("clip_mid", Curve::Clip { shape: 0.5 }, 2.0, None, Some(-13.79), -5.70),
        s("clip_hard", Curve::Clip { shape: 1.0 }, 2.0, None, Some(-12.91), -3.45),
        s("inflator", Curve::Inflator { curve: 0.0 }, 2.0, None, Some(-11.44), -8.61),
    ]
}

fn check_harmonic(name: &str, order: usize, got: f64, want: Option<f64>) {
    match want {
        Some(w) => assert!(
            (got - w).abs() < 0.05,
            "{name}: H{order} = {got:.2} dBc, pinned {w:.2}"
        ),
        None => assert!(got < ABSENT_DBC, "{name}: H{order} = {got:.2} dBc should be absent"),
    }
}

#[test]
fn harmonic_signatures_are_pinned() {
    for sig in signatures() {
        let spec = amplitude_spectrum(&render(sig.curve, Aa::Adaa, 4, 1000.0, sig.peak));
        let h = harmonics_dbc(&spec, bin(1000.0), 9);
        let name = format!("{} @ peak {}", sig.name, sig.peak);
        check_harmonic(&name, 2, h[2], sig.h2);
        check_harmonic(&name, 3, h[3], sig.h3);
        let decay = decay_db_per_order(&h, 2..=9, ABSENT_DBC);
        assert!(
            (decay - sig.decay).abs() < 0.05,
            "{name}: decay {decay:.2} dB/order, pinned {:.2}",
            sig.decay
        );
        // Every series falls: at least the ~3 dB/order of a hard clip.
        assert!(decay < -3.0, "{name}: decay {decay:.2}");
    }
}

#[test]
fn even_curves_are_h2_dominant_and_odd_curves_have_no_even_harmonics() {
    for sig in signatures() {
        let spec = amplitude_spectrum(&render(sig.curve, Aa::Adaa, 4, 1000.0, sig.peak));
        let h = harmonics_dbc(&spec, bin(1000.0), 9);
        let name = format!("{} @ peak {}", sig.name, sig.peak);
        if sig.curve.is_dc_safe() {
            for k in [2, 4, 6, 8] {
                assert!(h[k] < ABSENT_DBC, "{name}: odd curve has H{k} = {:.1}", h[k]);
            }
        } else {
            assert!(h[2] > h[3], "{name}: H2 {:.1} ≤ H3 {:.1}", h[2], h[3]);
        }
        if matches!(sig.curve, Curve::Warm { .. }) {
            for k in [3, 5, 7, 9] {
                assert!(h[k] < ABSENT_DBC, "{name}: even-only curve has H{k} = {:.1}", h[k]);
            }
        }
    }
}

/// Fourier sine coefficient `b_n` of `tanh(A·sin θ)`, by the periodic
/// trapezoid rule (spectrally accurate for a smooth periodic integrand).
fn tanh_series(a: f64, n: usize) -> f64 {
    const M: usize = 8192;
    let mut acc = 0.0;
    for i in 0..M {
        let th = std::f64::consts::TAU * i as f64 / M as f64;
        acc += (a * th.sin()).tanh() * (n as f64 * th).sin();
    }
    2.0 * acc / M as f64
}

#[test]
fn tanh_matches_the_analytic_series() {
    // 100 Hz: the series is far below the noise floor long before
    // Nyquist, so the 1× render has no aliasing to speak of, and the
    // ADAA render adds only its tiny lowpass.
    for drive in [1.0, 2.0, 4.0] {
        let b1 = tanh_series(drive, 1);
        for aa in [Aa::Naive, Aa::Adaa] {
            let spec = amplitude_spectrum(&render(Curve::Tanh, aa, 1, 100.0, drive));
            let f0 = bin(100.0);
            assert!(
                (db(spec[f0]) - db(b1)).abs() < 0.001,
                "{aa:?} drive {drive}: H1 {:.5} vs analytic {b1:.5}",
                spec[f0]
            );
            for n in [3, 5, 7] {
                let want = db(tanh_series(drive, n).abs() / b1);
                let got = db(spec[n * f0] / spec[f0]);
                assert!(
                    (got - want).abs() < 0.01,
                    "{aa:?} drive {drive}: H{n} {got:.3} dBc vs analytic {want:.3}"
                );
            }
        }
    }
}

/// The aliasing floor (strongest non-harmonic line) for a 5 kHz sine at
/// 0 dBFS with +12 dB drive (driven peak 4), per curve, first-order ADAA
/// inside the IIR oversampler at 4× and at 8× (a 2× `Oversampler`
/// wrapped around a 4× one). Pinned to ±1 dB so a regression shows up as
/// a number.
///
/// | curve      | 4× + ADAA | 8× + ADAA |
/// |------------|-----------|-----------|
/// | tanh       | −139.7    | −146.7    |
/// | tube       | −138.3    | −138.3    |
/// | warm       | −138.9    | −138.6    |
/// | console    |  −99.5    | −122.5    |
/// | clip_soft  |  −94.6    | −120.4    |
/// | clip_mid   |  −91.3    | −112.1    |
/// | clip_hard  |  −73.0    |  −90.1    |
/// | inflator   |  −80.1    | −104.8    |
///
/// The −90 dBc target holds at 4× for everything but the hard clipper
/// and the inflator (both flatten with a slope corner at full scale);
/// those two need the 8× cascade. ADAA alone at 1× reaches only
/// −20…−50 dBc at this (extreme) drive and frequency.
#[test]
fn aliasing_floor_at_plus_12_db_drive() {
    let table: [(&str, Curve, f64, f64); 8] = [
        ("tanh", Curve::Tanh, -139.7, -146.7),
        ("tube", Curve::Tube { bias: 0.5 }, -138.3, -138.3),
        ("warm", Curve::Warm { amount: 1.0 }, -138.9, -138.6),
        ("console", Curve::Console { drive: 1.0 }, -99.5, -122.5),
        ("clip_soft", Curve::Clip { shape: 0.0 }, -94.6, -120.4),
        ("clip_mid", Curve::Clip { shape: 0.5 }, -91.3, -112.1),
        ("clip_hard", Curve::Clip { shape: 1.0 }, -73.0, -90.1),
        ("inflator", Curve::Inflator { curve: 0.0 }, -80.1, -104.8),
    ];
    let f0 = bin(5000.0);
    for (name, curve, want4, want8) in table {
        let at = |aa, ratio| {
            alias_floor_dbc(&amplitude_spectrum(&render(curve, aa, ratio, 5000.0, 4.0)), f0)
        };
        let got4 = at(Aa::Adaa, 4);
        let got8 = at(Aa::Adaa, 8);
        assert!((got4 - want4).abs() < 1.0, "{name}: 4× ADAA floor {got4:.1}, pinned {want4}");
        assert!((got8 - want8).abs() < 1.0, "{name}: 8× ADAA floor {got8:.1}, pinned {want8}");
        let best = got4.min(got8);
        assert!(best <= -90.0, "{name}: best floor {best:.1} dBc misses −90");
        // ADAA helps at every rate.
        for ratio in [1, 4] {
            let naive = at(Aa::Naive, ratio);
            let adaa = at(Aa::Adaa, ratio);
            assert!(adaa < naive - 3.0, "{name} {ratio}×: ADAA {adaa:.1} vs naive {naive:.1}");
        }
    }
}

fn all_curves() -> Vec<Curve> {
    let mut v: Vec<Curve> = curves().into_iter().map(|(_, c)| c).collect();
    v.extend([
        Curve::Tube { bias: -0.8 },
        Curve::Warm { amount: 0.3 },
        Curve::Console { drive: 4.0 },
        Curve::Clip { shape: 0.9 },
        Curve::Inflator { curve: -0.5 },
        Curve::Inflator { curve: 0.5 },
    ]);
    v
}

#[test]
fn antiderivatives_match_their_curves() {
    for c in all_curves() {
        let mut u = -6.0f64;
        while u <= 6.0 {
            let h = 1e-5;
            let numeric = (c.antiderivative(u + h) - c.antiderivative(u - h)) / (2.0 * h);
            let f = c.eval(u);
            assert!((numeric - f).abs() < 1e-5, "{c:?} at {u}: F' {numeric} vs f {f}");
            u += 0.01;
        }
        assert_eq!(c.eval(0.0), 0.0, "{c:?} must pass through the origin");
    }
}

#[test]
fn clipper_and_inflator_are_bounded_and_unity_below_the_knee() {
    for shape in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
        let c = Curve::Clip { shape };
        for i in -400..=400 {
            let u = i as f64 * 0.01;
            let y = c.eval(u);
            assert!(y.abs() <= 1.0 + 1e-12, "{c:?}({u}) = {y}");
            if u.abs() <= shape as f64 {
                assert_eq!(y, u, "{c:?} must be linear below its knee");
            }
        }
        assert_eq!(c.eval(2.0), 1.0);
    }
    for curve in [-0.5f32, 0.0, 0.5] {
        let c = Curve::Inflator { curve };
        assert!((c.eval(1.0) - 1.0).abs() < 1e-12);
        let slope = (c.eval(1.0) - c.eval(1.0 - 1e-6)) / 1e-6;
        assert!(slope.abs() < 1e-4, "{c:?}: slope at 1 is {slope}");
        assert!((c.slope_at_zero() - (1.5 + curve as f64)).abs() < 1e-12);
        for i in -300..=300 {
            assert!(c.eval(i as f64 * 0.01).abs() <= 1.0 + 1e-12);
        }
    }
}

#[test]
fn asymmetric_curves_need_the_dc_blocker_and_odd_curves_do_not() {
    for c in all_curves() {
        let out = render(c, Aa::Adaa, 1, 1000.0, 1.0);
        let mean = out.iter().map(|&v| v as f64).sum::<f64>() / out.len() as f64;
        if c.is_dc_safe() {
            assert!(mean.abs() < 1e-6, "{c:?}: DC {mean}");
        } else {
            assert!(mean.abs() > 1e-3, "{c:?}: claims to need a DC blocker but has DC {mean}");
            let mut blocker = resonance_dsp::DcBlocker::new(5.0, SR as f32);
            let x = sine(1000.0, 1.0, SR, 48_000);
            let mut adaa = Adaa1::new();
            let y: Vec<f32> =
                x.iter().map(|&s| blocker.process(adaa.process(&c, s as f32))).collect();
            let tail = &y[38_400..];
            let m = tail.iter().map(|&v| v as f64).sum::<f64>() / tail.len() as f64;
            assert!(m.abs() < 1e-4, "{c:?}: DC after the blocker {m}");
        }
    }
}

#[test]
fn identity_curves_pass_through_bit_exact() {
    let x = sine(3000.0, 0.9, SR, 4800);
    for c in [Curve::Console { drive: 0.0 }, Curve::Warm { amount: 0.0 }] {
        assert!(c.is_identity());
        let mut adaa = Adaa1::new();
        for &s in &x {
            let s = s as f32;
            assert_eq!(adaa.process(&c, s).to_bits(), s.to_bits(), "{c:?}");
        }
    }
    assert!(!Curve::Tanh.is_identity());
}

#[test]
fn extreme_inputs_never_produce_nan_or_inf() {
    let nasty = [
        0.0f32,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        1e30,
        -1e30,
        f32::MAX,
        f32::MIN,
        1e-40,
        -1e-40,
        1e4,
        -1e4,
        1e-6,
        f32::NAN,
        0.5,
    ];
    for c in all_curves() {
        let mut adaa = Adaa1::new();
        for _ in 0..3 {
            for &u in &nasty {
                let y = adaa.process(&c, u);
                assert!(y.is_finite(), "{c:?}: ADAA({u}) = {y}");
                let z = c.eval(u as f64);
                assert!(z.is_finite(), "{c:?}: eval({u}) = {z}");
            }
        }
        // A curve change mid-stream (a parameter ramp) on a tiny step
        // must not divide a stale antiderivative by it.
        let mut adaa = Adaa1::new();
        adaa.process(&c, 0.3);
        let y = adaa.process(&Curve::Clip { shape: 0.2 }, 0.300_01);
        assert!(y.abs() < 1.0, "{c:?} → clip on a tiny step gave {y}");
    }
}

#[test]
fn nan_and_out_of_range_parameters_are_clamped() {
    for c in [
        Curve::Tube { bias: f32::NAN },
        Curve::Tube { bias: 1e9 },
        Curve::Warm { amount: f32::NAN },
        Curve::Warm { amount: 1e9 },
        Curve::Console { drive: f32::NAN },
        Curve::Console { drive: 1e9 },
        Curve::Clip { shape: f32::NAN },
        Curve::Clip { shape: -3.0 },
        Curve::Inflator { curve: f32::NAN },
        Curve::Inflator { curve: 7.0 },
    ] {
        let mut adaa = Adaa1::new();
        for &u in &[0.0f32, 0.5, -3.0, 1e30, 0.25] {
            assert!(adaa.process(&c, u).is_finite(), "{c:?} at {u}");
            assert!(c.eval(u as f64).is_finite() && c.slope_at_zero().is_finite(), "{c:?}");
        }
    }
    assert!(Curve::Tube { bias: f32::NAN }.is_dc_safe());
    assert!(Curve::Console { drive: f32::NAN }.is_identity());
}
