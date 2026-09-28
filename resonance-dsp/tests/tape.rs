//! Tape filters: head-bump frequency tracks speed, HF loss deepens with
//! level, flutter at 0 is a bit-exact bypass, and nothing produces NaN.

mod common;

use common::*;
use resonance_dsp::tape::{head_bump_hz, hf_loss_corner_hz};
use resonance_dsp::{Flutter, HeadBump, HfLoss};

const FS: f32 = 48_000.0;

/// Frequency of the largest (or, with `min`, smallest) magnitude of `h`
/// on a 1/200-octave sweep over `lo..hi`.
fn extremum(h: &HeadBump, lo: f32, hi: f32, min: bool) -> f32 {
    let mut best = (lo, h.magnitude(lo, FS));
    let mut f = lo;
    while f <= hi {
        let m = h.magnitude(f, FS);
        if (min && m < best.1) || (!min && m > best.1) {
            best = (f, m);
        }
        f *= 2f32.powf(1.0 / 200.0);
    }
    best.0
}

#[test]
fn head_bump_frequency_is_proportional_to_speed() {
    assert!((50.0..=60.0).contains(&head_bump_hz(15.0)), "{}", head_bump_hz(15.0));
    assert!((100.0..=120.0).contains(&head_bump_hz(30.0)), "{}", head_bump_hz(30.0));
    assert!((head_bump_hz(7.5) * 2.0 - head_bump_hz(15.0)).abs() < 1e-3);
    for speed in [7.5f32, 15.0, 30.0] {
        let mut hb = HeadBump::new();
        hb.set(FS, speed, 3.0, -1.5);
        let f = head_bump_hz(speed);
        let peak = extremum(&hb, f / 3.0, f * 1.5, false);
        let dip = extremum(&hb, f * 1.5, f * 4.0, true);
        // Each filter pushes the other's extremum away a little (the dip
        // lands ≈ 11 % above its design frequency at any speed).
        assert!((peak / f - 1.0).abs() < 0.10, "{speed} ips: peak at {peak:.1} Hz, design {f:.1}");
        assert!((dip / (2.0 * f) - 1.0).abs() < 0.15, "{speed} ips: dip at {dip:.1} Hz, design {:.1}", 2.0 * f);
        let peak_db = 20.0 * hb.magnitude(peak, FS).log10();
        let dip_db = 20.0 * hb.magnitude(dip, FS).log10();
        assert!(peak_db > 2.0 && peak_db < 3.5, "{speed} ips: peak {peak_db:.2} dB");
        // The bump's skirt fills in part of the −1.5 dB dip.
        assert!(dip_db < -0.5, "{speed} ips: dip {dip_db:.2} dB");
        // Far above the bump the stage is flat.
        let hf_db = 20.0 * hb.magnitude(5_000.0, FS).log10();
        assert!(hf_db.abs() < 0.05, "{speed} ips: {hf_db:.3} dB at 5 kHz");
    }
}

#[test]
fn head_bump_processes_what_its_magnitude_says() {
    let mut hb = HeadBump::new();
    hb.set(FS, 15.0, 3.0, -1.5);
    let f = 55.0;
    let x = sine(f as f64, 0.5, SR, 96_000);
    let y: Vec<f32> = x.iter().map(|&s| hb.process(s as f32)).collect();
    let tail = &y[48_000..];
    let rms = (tail.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
    let got = db(rms / (0.5 / 2f64.sqrt()));
    let want = 20.0 * hb.magnitude(f, FS).log10() as f64;
    assert!((got - want).abs() < 0.05, "processed {got:.3} dB vs magnitude {want:.3} dB");
}

#[test]
fn zero_gain_stages_are_bit_exact() {
    let x = noise(4_800, 0.9, 1);
    let mut hb = HeadBump::new();
    hb.set(FS, 15.0, 0.0, 0.0);
    let mut hf = HfLoss::new();
    hf.set_amounts(0.0, 0.0, 0.25);
    for &s in &x {
        assert_eq!(hb.process(s).to_bits(), s.to_bits());
        assert_eq!(hf.process(s).to_bits(), s.to_bits());
    }
}

/// Steady-state gain (dB) of `hf` for a sine at `freq`, `amp`.
fn hf_gain_db(freq: f64, amp: f64) -> f64 {
    let mut hf = HfLoss::new();
    hf.set_corner(FS, hf_loss_corner_hz(15.0));
    hf.set_times(FS, 1.0, 50.0);
    hf.set_amounts(-0.5, -6.0, 0.25);
    let x = sine(freq, amp, SR, 48_000);
    let y: Vec<f32> = x.iter().map(|&s| hf.process(s as f32)).collect();
    let spec = amplitude_spectrum(&y[38_400..]);
    let b = (freq / 5.0).round() as usize;
    db(spec[b] / amp)
}

#[test]
fn hf_loss_deepens_with_level_and_spares_the_lows() {
    // The shelf is first order with its corner at 10 kHz, so at 12 kHz
    // it realises a little under half of the cut in dB.
    let quiet = hf_gain_db(12_000.0, 0.01);
    let mid = hf_gain_db(12_000.0, 0.1);
    let loud = hf_gain_db(12_000.0, 1.0);
    // Quiet: only (part of) the static −0.5 dB.
    assert!(quiet < 0.0 && quiet > -0.6, "quiet 12 kHz at {quiet:.2} dB");
    // Louder: the dynamic cut deepens monotonically.
    assert!(mid < quiet - 0.2, "−20 dB 12 kHz at {mid:.2} dB vs quiet {quiet:.2}");
    assert!(loud < mid - 0.5, "loud 12 kHz at {loud:.2} dB vs −20 dB {mid:.2}");
    assert!(loud < -1.2, "loud 12 kHz at {loud:.2} dB");
    // A loud 100 Hz tone barely touches the high band, so it passes.
    let bass = hf_gain_db(100.0, 1.0);
    assert!(bass.abs() < 0.1, "loud 100 Hz at {bass:.3} dB");
    assert!((hf_loss_corner_hz(30.0) - 2.0 * hf_loss_corner_hz(15.0)).abs() < 1e-3);
}

#[test]
fn flutter_at_zero_is_a_bit_exact_bypass() {
    let x = noise(9_600, 0.9, 5);
    let mut fl = Flutter::new(FS);
    fl.set_amount(0.0);
    for &s in &x {
        assert_eq!(fl.process(s).to_bits(), s.to_bits());
    }
    // Negative and NaN amounts clamp to the bypass too.
    for a in [-1.0f32, f32::NAN] {
        let mut fl = Flutter::new(FS);
        fl.set_amount(a);
        for &s in &x[..100] {
            assert_eq!(fl.process(s).to_bits(), s.to_bits());
        }
    }
}

#[test]
fn flutter_modulates_deterministically_and_keeps_level() {
    let x = sine(1_000.0, 0.5, SR, 96_000);
    let run = || {
        let mut fl = Flutter::new(FS);
        fl.set_amount(1.0);
        x.iter().map(|&s| fl.process(s as f32)).collect::<Vec<f32>>()
    };
    let a = run();
    let b = run();
    assert_eq!(a, b, "two instances with the same settings modulate identically");
    let tail = &a[9_600..];
    let rms = (tail.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
    let level = db(rms / (0.5 / 2f64.sqrt()));
    assert!(level.abs() < 0.1, "flutter changed the level by {level:.3} dB");
    // It is not a plain delay: the 1 kHz line smears into sidebands.
    let spec = amplitude_spectrum(&a[9_600..57_600]);
    let side = (1..=20).map(|k| spec[(1_000.0 / 1.0) as usize + k]).fold(0.0f64, f64::max);
    assert!(db(side / spec[1_000]) > -60.0, "no wow/flutter sidebands");
}

#[test]
fn extreme_inputs_never_produce_nan() {
    let nasty = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30, -1e30, 1e-40, 0.0, 1.0];
    let mut hb = HeadBump::new();
    hb.set(FS, 15.0, 3.0, -1.5);
    let mut hf = HfLoss::new();
    hf.set_amounts(-1.0, -6.0, 0.25);
    let mut fl = Flutter::new(FS);
    fl.set_amount(1.0);
    for _ in 0..200 {
        for &s in &nasty {
            let y = fl.process(s);
            assert!(y.is_finite(), "flutter({s}) = {y}");
        }
    }
    // The IIR filters are linear: a finite input stays finite.
    for &s in &[1e30f32, -1e30, 1e-40, 0.0] {
        for _ in 0..100 {
            assert!(hb.process(s).is_finite());
            assert!(hf.process(s).is_finite());
        }
    }
}
