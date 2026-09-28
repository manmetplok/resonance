//! LF-weighted drive: the emphasis pair cancels at small signal, bass
//! distorts before treble, the sub-sonic high-pass has its small bump,
//! and extreme inputs stay finite.

mod common;

use common::*;
use resonance_dsp::saturate::Curve;
use resonance_dsp::{Biquad, EmphasisPair, LfWeightedDrive};

const FS: f32 = 48_000.0;

#[test]
fn emphasis_pair_is_an_exact_inverse_at_small_signal() {
    let x = noise(48_000, 0.5, 3);
    for gain in [-9.0f32, 6.0, 12.0] {
        let mut e = EmphasisPair::new();
        e.set(FS, 300.0, gain);
        let mut worst = 0.0f32;
        for &s in &x {
            let p = e.pre(s);
            let y = e.post(p);
            worst = worst.max((y - s).abs());
        }
        // f32 rounding in two low-corner biquads: ≈ −85 dB of noise.
        assert!(worst < 1e-4, "gain {gain}: post(pre(x)) off by {worst:e}");
    }
    let mut off = EmphasisPair::new();
    off.set(FS, 300.0, 0.0);
    let p = off.pre(0.3);
    assert_eq!(off.post(p).to_bits(), 0.3f32.to_bits());
}

/// H3 (dBc) of a sine at `freq` through the stage.
fn h3_dbc(stage: &mut LfWeightedDrive, freq: f64, amp: f64) -> f64 {
    let x = sine(freq, amp, SR, 19_200);
    let y: Vec<f32> = x.iter().map(|&s| stage.process(s as f32)).collect();
    let spec = amplitude_spectrum(&y[9_600..]);
    let b = (freq / 5.0).round() as usize;
    harmonics_dbc(&spec, b, 3)[3]
}

#[test]
fn bass_saturates_before_treble() {
    let make = |gain_db| {
        let mut s = LfWeightedDrive::new();
        s.set_curve(Curve::Tanh);
        s.set_drive(2.0);
        s.set_emphasis(FS, 300.0, gain_db);
        s
    };
    // Flat: the same H3 at 100 Hz and 2 kHz (a memoryless curve).
    let flat_lo = h3_dbc(&mut make(0.0), 100.0, 0.25);
    let flat_hi = h3_dbc(&mut make(0.0), 2_000.0, 0.25);
    assert!((flat_lo - flat_hi).abs() < 0.5, "flat: {flat_lo:.1} vs {flat_hi:.1}");
    // +9 dB LF emphasis: the bass hits the curve ~9 dB hotter, so its H3
    // rises by well over 10 dB while the 2 kHz tone barely moves.
    let lo = h3_dbc(&mut make(9.0), 100.0, 0.25);
    let hi = h3_dbc(&mut make(9.0), 2_000.0, 0.25);
    assert!(lo > flat_lo + 10.0, "emphasised 100 Hz H3 {lo:.1} vs flat {flat_lo:.1}");
    assert!((hi - flat_hi).abs() < 1.5, "emphasised 2 kHz H3 {hi:.1} vs flat {flat_hi:.1}");
    // Negative emphasis weights the drive toward the highs instead.
    let lo_neg = h3_dbc(&mut make(-9.0), 100.0, 0.25);
    assert!(lo_neg < flat_lo - 10.0, "de-emphasised 100 Hz H3 {lo_neg:.1}");
}

#[test]
fn small_signal_gain_is_unity_at_any_drive() {
    for drive in [0.5f32, 1.0, 4.0] {
        let mut s = LfWeightedDrive::new();
        s.set_drive(drive);
        s.set_emphasis(FS, 300.0, 9.0);
        let x = sine(200.0, 1e-4, SR, 19_200);
        let y: Vec<f32> = x.iter().map(|&v| s.process(v as f32)).collect();
        let spec = amplitude_spectrum(&y[9_600..]);
        let g = db(spec[40] / 1e-4);
        assert!(g.abs() < 0.01, "drive {drive}: small-signal gain {g:.4} dB");
    }
}

#[test]
fn subsonic_high_pass_has_a_small_bump_and_removes_dc() {
    // The filter the stage uses, measured directly.
    let mut hp = Biquad::identity();
    hp.set_high_pass(FS, 20.0, LfWeightedDrive::SUBSONIC_Q);
    let peak = (20..200)
        .map(|f| 20.0 * hp.magnitude(f as f32, FS).log10())
        .fold(f32::MIN, f32::max);
    assert!(peak > 0.3 && peak < 1.0, "bump {peak:.2} dB");
    assert!(20.0 * hp.magnitude(5.0, FS).log10() < -20.0);

    // An asymmetric curve's DC is gone at the output.
    let mut s = LfWeightedDrive::new();
    s.set_curve(Curve::Tube { bias: 0.5 });
    s.set_drive(4.0);
    s.set_subsonic(FS, 20.0, LfWeightedDrive::SUBSONIC_Q);
    let x = sine(1_000.0, 0.5, SR, 48_000);
    let y: Vec<f32> = x.iter().map(|&v| s.process(v as f32)).collect();
    let tail = &y[38_400..];
    let mean = tail.iter().map(|&v| v as f64).sum::<f64>() / tail.len() as f64;
    assert!(mean.abs() < 1e-4, "DC {mean}");
}

#[test]
fn hf_resonance_off_by_default_and_audible_when_set() {
    let mut s = LfWeightedDrive::new();
    s.set_drive(0.01);
    let mut r = LfWeightedDrive::new();
    r.set_drive(0.01);
    r.set_hf_resonance(FS, 16_000.0, 1.5);
    let x = sine(16_000.0, 0.01, SR, 19_200);
    let a: Vec<f32> = x.iter().map(|&v| s.process(v as f32)).collect();
    let b: Vec<f32> = x.iter().map(|&v| r.process(v as f32)).collect();
    let bin = 16_000 / 5;
    let lift = db(amplitude_spectrum(&b[9_600..])[bin] / amplitude_spectrum(&a[9_600..])[bin]);
    assert!((lift - 1.5).abs() < 0.1, "HF resonance lift {lift:.2} dB");
}

#[test]
fn extreme_inputs_never_produce_nan() {
    let mut s = LfWeightedDrive::new();
    s.set_drive(8.0);
    s.set_emphasis(FS, 300.0, 12.0);
    s.set_subsonic(FS, 20.0, 0.9);
    s.set_hf_resonance(FS, 18_000.0, 2.0);
    for c in [Curve::Tanh, Curve::Tube { bias: 0.5 }, Curve::Clip { shape: 1.0 }] {
        s.set_curve(c);
        for &x in &[1e30f32, -1e30, f32::MAX, 1e-40, 0.0, 1.0, -1.0] {
            for _ in 0..50 {
                let y = s.process(x);
                assert!(y.is_finite(), "{c:?}: {x} → {y}");
            }
        }
    }
}
