//! The stereo decorrelators: the velvet-noise pure-side widener keeps the
//! mono sum, the all-pass cascade keeps its mono ripple small, and both
//! actually decorrelate.

mod common;

use common::*;
use resonance_dsp::decorrelate::{erb_number, erb_number_to_hz};
use resonance_dsp::{AllpassDecorrelator, VelvetDecorrelator};
use rustfft::{num_complex::Complex, FftPlanner};

const FS: f32 = 48_000.0;

/// Mono-sum magnitude `|(H_L + H_R)/2|` of the cascade in dB, per bin up
/// to `hi_hz`, for a mono input.
fn allpass_mono_db(ap: &mut AllpassDecorrelator, n: usize, hi_hz: f32) -> Vec<(f32, f64)> {
    let mut l = Vec::with_capacity(n);
    let mut r = Vec::with_capacity(n);
    for i in 0..n {
        let x = if i == 0 { 1.0 } else { 0.0 };
        let (a, b) = ap.process(x, x);
        l.push(Complex::new(a as f64, 0.0));
        r.push(Complex::new(b as f64, 0.0));
    }
    let fft = FftPlanner::new().plan_fft_forward(n);
    fft.process(&mut l);
    fft.process(&mut r);
    (1..n / 2)
        .map(|k| (k as f32 * FS / n as f32, k))
        .filter(|&(f, _)| f >= 20.0 && f <= hi_hz)
        .map(|(f, k)| (f, db(((l[k] + r[k]) * 0.5).norm())))
        .collect()
}

#[test]
fn velvet_widening_leaves_the_mono_sum_unchanged() {
    // Mono noise, a stereo mix and a sine, at several amounts and with
    // and without the band focus.
    let n = 48_000;
    let a = noise(n, 0.8, 7);
    let b = noise(n, 0.5, 11);
    let sine: Vec<f32> = common::sine(440.0, 0.9, SR, n).iter().map(|&v| v as f32).collect();
    let cases: [(&str, &[f32], &[f32]); 3] =
        [("mono noise", &a, &a), ("stereo noise", &a, &b), ("sine vs noise", &sine, &b)];
    for (name, l, r) in cases {
        for amount in [0.25f32, 0.7, 1.0] {
            for focus in [(0.0, 0.0), (150.0, 0.0), (300.0, 8_000.0)] {
                let mut d = VelvetDecorrelator::new(FS, 1);
                d.set_amount(amount);
                d.set_focus(FS, focus.0, focus.1);
                let mut worst = 0.0f32;
                let mut side_energy = 0.0f64;
                for i in 0..n {
                    let (ol, or) = d.process(l[i], r[i]);
                    let before = l[i] + r[i];
                    let after = ol + or;
                    // A few ULPs of the largest operand: (l + s) and
                    // (r − s) each round once, then the sum rounds.
                    let scale = l[i].abs().max(r[i].abs()).max((ol - l[i]).abs()).max(1e-30);
                    worst = worst.max((after - before).abs() / scale);
                    side_energy += ((ol - l[i]) as f64).powi(2);
                }
                assert!(
                    worst <= 4.0 * f32::EPSILON,
                    "{name} a={amount} focus={focus:?}: mono sum moved by {worst:e} (relative)"
                );
                assert!(side_energy > 1.0, "{name} a={amount}: no side was added");
            }
        }
    }
}

#[test]
fn velvet_amount_zero_is_an_exact_passthrough() {
    let l = noise(4_800, 0.9, 3);
    let r = noise(4_800, 0.9, 4);
    let mut d = VelvetDecorrelator::new(FS, 99);
    d.set_focus(FS, 150.0, 0.0);
    for i in 0..l.len() {
        let (a, b) = d.process(l[i], r[i]);
        assert_eq!((a.to_bits(), b.to_bits()), (l[i].to_bits(), r[i].to_bits()));
    }
}

#[test]
fn velvet_decorrelates_a_mono_source() {
    let n = 96_000;
    let x = noise(n, 0.5, 5);
    for (amount, want) in [(1.0f32, 0.0f64), (0.5, 0.6)] {
        let mut d = VelvetDecorrelator::new(FS, 1);
        d.set_amount(amount);
        let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for &s in &x {
            let (a, b) = d.process(s, s);
            l.push(a);
            r.push(b);
        }
        // (1 − a²)/(1 + a²) for a unit-energy D uncorrelated with M.
        let rho = correlation(&l[4_800..], &r[4_800..]);
        assert!((rho - want).abs() < 0.08, "amount {amount}: correlation {rho:.3}, expected ≈ {want}");
    }
}

#[test]
fn velvet_design_is_deterministic_and_sparse() {
    let a = VelvetDecorrelator::new(FS, 42);
    assert_eq!(a.tap_count(), 45, "30 ms at 1500 impulses/s");
    let x = noise(2_000, 0.5, 8);
    let run = |seed| {
        let mut d = VelvetDecorrelator::new(FS, seed);
        d.set_amount(1.0);
        x.iter().map(|&s| d.side(s).to_bits()).collect::<Vec<_>>()
    };
    assert_eq!(run(42), run(42), "same seed, same filter");
    assert_ne!(run(42), run(43), "a different seed is a different filter");
}

#[test]
fn velvet_focus_keeps_the_bass_dry() {
    // The focus high-pass acts on the side component only: at 60 Hz a
    // 150 Hz second-order high-pass takes 16 dB off it, at 2 kHz nothing.
    let side_level = |freq: f64, focus_hz: f32| {
        let mut d = VelvetDecorrelator::new(FS, 1);
        d.set_amount(1.0);
        d.set_focus(FS, focus_hz, 0.0);
        let x = common::sine(freq, 0.5, SR, 48_000);
        let s: Vec<f32> = x.iter().map(|&v| d.side(v as f32)).collect();
        let tail = &s[9_600..];
        (tail.iter().map(|&a| (a as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt()
    };
    let bass = db(side_level(60.0, 150.0) / side_level(60.0, 0.0));
    let mid = db(side_level(2_000.0, 150.0) / side_level(2_000.0, 0.0));
    assert!((bass + 16.0).abs() < 1.0, "60 Hz side moved by {bass:.1} dB");
    assert!(mid.abs() < 0.1, "2 kHz side moved by {mid:.2} dB");
}

/// Noise correlation of the cascade's two outputs for a mono input.
fn allpass_correlation(ap: &mut AllpassDecorrelator) -> f64 {
    let x = noise(96_000, 0.5, 9);
    let (mut l, mut r) = (Vec::with_capacity(x.len()), Vec::with_capacity(x.len()));
    for &s in &x {
        let (a, b) = ap.process(s, s);
        l.push(a);
        r.push(b);
    }
    correlation(&l[4_800..], &r[4_800..])
}

#[test]
fn allpass_cascade_mono_ripple_stays_under_2_db() {
    // Measured at the default spread (0.2 of an ERB step): the mono sum
    // dips by at most 1.4 dB, for a noise correlation of about 0.70.
    for sections in [30, 40, 50] {
        let mut ap = AllpassDecorrelator::default();
        ap.configure(
            FS,
            sections,
            AllpassDecorrelator::DEFAULT_LOW_HZ,
            AllpassDecorrelator::DEFAULT_HIGH_HZ,
            AllpassDecorrelator::DEFAULT_SPREAD,
        );
        assert_eq!(ap.sections(), sections);
        let m = allpass_mono_db(&mut ap, 1 << 16, 20_000.0);
        let lo = m.iter().map(|p| p.1).fold(f64::MAX, f64::min);
        let hi = m.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        assert!(
            hi - lo < 2.0 && lo > -2.0,
            "{sections} sections: mono sum spans {lo:.2}..{hi:.2} dB"
        );
        assert!(hi < 0.01, "{sections} sections: an all-pass pair cannot boost the mono sum ({hi:.3} dB)");
        let rho = allpass_correlation(&mut ap);
        assert!(rho < 0.8, "{sections} sections: correlation {rho:.3}, not decorrelated");
    }
}

#[test]
fn allpass_cascade_keeps_each_channel_flat() {
    let mut ap = AllpassDecorrelator::new(FS);
    let n = 1 << 16;
    let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let x = if i == 0 { 1.0 } else { 0.0 };
        let (a, b) = ap.process(x, x);
        l.push(a);
        r.push(b);
    }
    for (name, h) in [("left", l), ("right", r)] {
        let spec = amplitude_spectrum(&h);
        // `amplitude_spectrum` doubles non-DC bins (it reads sine peaks),
        // so a unit impulse reads 2/n per bin.
        for (k, &a) in spec.iter().enumerate().skip(1).take(n / 2 - 1) {
            let g = db(a * n as f64 / 2.0);
            assert!(g.abs() < 0.01, "{name}: bin {k} at {g:.3} dB");
        }
    }
}

#[test]
fn allpass_spread_zero_is_identical_per_side_and_bass_stays_in_phase() {
    let mut ap = AllpassDecorrelator::default();
    ap.configure(FS, 40, 150.0, 16_000.0, 0.0);
    let x = noise(4_800, 0.5, 2);
    for &s in &x {
        let (a, b) = ap.process(s, s);
        assert_eq!(a.to_bits(), b.to_bits());
    }
    // Below the first section the phase difference fades out: the mono
    // sum at 40 Hz is within 0.1 dB at the default spread.
    let mut ap = AllpassDecorrelator::new(FS);
    let m = allpass_mono_db(&mut ap, 1 << 16, 40.0);
    let (_, low) = m.last().copied().unwrap();
    assert!(low > -0.1, "40 Hz mono sum at {low:.2} dB");
    // Zero sections bypass.
    let mut off = AllpassDecorrelator::default();
    assert_eq!(off.process(0.25, -0.5), (0.25, -0.5));
}

#[test]
fn erb_scale_round_trips() {
    for f in [50.0f32, 150.0, 1_000.0, 4_000.0, 16_000.0] {
        let back = erb_number_to_hz(erb_number(f));
        assert!((back - f).abs() / f < 1e-4, "{f} → {back}");
    }
    assert!((erb_number(1_000.0) - 15.62).abs() < 0.01);
}

#[test]
fn extreme_inputs_never_produce_nan() {
    let mut v = VelvetDecorrelator::new(FS, 3);
    v.set_amount(1.0);
    v.set_focus(FS, 150.0, 12_000.0);
    let mut ap = AllpassDecorrelator::new(FS);
    for _ in 0..100 {
        for &x in &[1e30f32, -1e30, 1e-40, 0.0, 1.0] {
            let (a, b) = v.process(x, -x);
            assert!(a.is_finite() && b.is_finite(), "velvet({x})");
            let (a, b) = ap.process(x, x);
            assert!(a.is_finite() && b.is_finite(), "allpass({x})");
        }
        // A non-finite sample never enters the velvet delay line.
        assert!(v.side(f32::NAN).is_finite());
        assert!(v.side(f32::INFINITY).is_finite());
    }
}
