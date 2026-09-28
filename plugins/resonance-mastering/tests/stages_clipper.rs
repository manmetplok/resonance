//! The clipper before the limiter (warmth-width-depth.md §6.3): it takes
//! the expected number of dB off a peak, leaves material under its knee
//! alone, keeps its aliases ≤ −90 dBc at 8×, and is a wire when off.

use resonance_mastering::stages::clipper::{Clipper, ClipperConfig};
use resonance_mastering::ResonanceMastering;
use resonance_plugin::ResonancePlugin;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

const SR: f32 = 48_000.0;
const BLOCK: usize = 480;
const TAU: f64 = std::f64::consts::TAU;

fn sine(freq: f64, amp: f64, len: usize) -> Vec<f32> {
    (0..len)
        .map(|n| (amp * (TAU * freq * n as f64 / SR as f64).sin()) as f32)
        .collect()
}

fn run(cfg: &ClipperConfig, input: &[f32]) -> Vec<f32> {
    let mut c = Clipper::new(SR);
    let mut l = input.to_vec();
    let mut r = input.to_vec();
    for start in (0..l.len()).step_by(BLOCK) {
        let end = (start + BLOCK).min(l.len());
        c.process_stereo(&mut l[start..end], &mut r[start..end], cfg);
    }
    assert_eq!(l, r, "identical channels must clip identically");
    l
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-15).log10()
}

fn cfg(drive_db: f32, softness: f32) -> ClipperConfig {
    ClipperConfig {
        enabled: true,
        drive_db,
        softness,
    }
}

#[test]
fn hard_clip_takes_drive_db_off_a_full_scale_peak() {
    for drive in [1.0f32, 3.0, 6.0] {
        let out = run(&cfg(drive, 0.0), &sine(100.0, 1.0, 24_000));
        let got = db(peak(&out[4800..]) as f64);
        // The ceiling is −drive dBFS; the IIR half-bands ring a little
        // around the clipped corners.
        assert!(
            (got + drive as f64).abs() < 0.25,
            "drive {drive} dB: peak {got:.2} dBFS, want {:.2}",
            -drive
        );
    }
}

#[test]
fn material_under_the_knee_passes_at_unity() {
    // −12 dBFS into a 3 dB clip: nowhere near the −3 dBFS ceiling.
    let input = sine(440.0, 0.25, 24_000);
    let out = run(&cfg(3.0, 0.0), &input);
    let got = db(peak(&out[4800..]) as f64) - db(0.25);
    assert!(got.abs() < 0.02, "level moved {got:.3} dB");
}

#[test]
fn softer_shapes_distort_less_at_the_same_drive() {
    let input = sine(1000.0, 1.0, 48_000);
    let hard = harmonics_db(&run(&cfg(6.0, 0.0), &input)[24_000..], 1000.0);
    let soft = harmonics_db(&run(&cfg(6.0, 1.0), &input)[24_000..], 1000.0);
    // H3 relative to the fundamental.
    assert!(soft.h3 < hard.h3 - 3.0, "soft H3 {:.1} vs hard {:.1}", soft.h3, hard.h3);
}

struct Harmonics {
    h3: f64,
}

fn harmonics_db(x: &[f32], f0: f64) -> Harmonics {
    let spec = spectrum(x);
    let bin = |f: f64| spec[(f * x.len() as f64 / SR as f64).round() as usize];
    Harmonics {
        h3: db(bin(3.0 * f0) / bin(f0)),
    }
}

/// Magnitude spectrum of exactly one second of signal: 1 Hz bins, so a
/// tone on a whole number of Hz and every product of it land on a bin
/// with no leakage (rectangular window, periodic signal).
fn spectrum(x: &[f32]) -> Vec<f64> {
    let n = x.len();
    let mut buf: Vec<Complex<f64>> = x.iter().map(|&v| Complex::new(v as f64, 0.0)).collect();
    FftPlanner::new().plan_fft_forward(n).process(&mut buf);
    buf[..n / 2].iter().map(|c| c.norm()).collect()
}

/// The strongest component that is neither DC nor a harmonic of `f0`
/// below Nyquist, in dB relative to the fundamental.
fn worst_alias_dbc(x: &[f32], f0: usize) -> f64 {
    assert_eq!(x.len(), SR as usize, "needs exactly one second");
    let spec = spectrum(x);
    let fundamental = spec[f0];
    let mut worst = 0.0f64;
    for (hz, &m) in spec.iter().enumerate().skip(1) {
        if hz % f0 == 0 {
            continue;
        }
        worst = worst.max(m);
    }
    db(worst / fundamental)
}

/// The spec's measurement (warmth-width-depth.md §11): a 5 kHz sine at
/// 0 dBFS with +12 dB drive, hard clip. At 8× (2× around 4×) with ADAA
/// the strongest alias stays ≤ −90 dBc. A 5 kHz tone at 48 kHz folds
/// its harmonics onto whole kHz that are not multiples of 5 (25 k → 23 k,
/// 35 k → 13 k, 45 k → 3 k, …), so they are told apart exactly.
#[test]
fn hard_clip_alias_floor_at_8x() {
    let input = sine(5000.0, 1.0, 2 * SR as usize);
    let out = run(&cfg(12.0, 0.0), &input);
    let tail = &out[SR as usize..];
    let alias = worst_alias_dbc(tail, 5000);
    eprintln!("hard clip, 5 kHz, +12 dB drive, 8x: worst alias {alias:.1} dBc");
    assert!(alias <= -90.0, "worst alias {alias:.1} dBc, want <= -90");
    // And the clip really happened: H3 is large (a hard clip of a sine
    // driven 12 dB over the ceiling is close to a square wave).
    let spec = spectrum(tail);
    let h3 = db(spec[15_000] / spec[5_000]);
    assert!(h3 > -20.0, "H3 only {h3:.1} dBc: the tone was not clipped");
}

#[test]
fn soft_clip_alias_floor_at_8x() {
    let input = sine(5000.0, 1.0, 2 * SR as usize);
    let out = run(&cfg(12.0, 0.5), &input);
    let alias = worst_alias_dbc(&out[SR as usize..], 5000);
    eprintln!("mid-soft clip, 5 kHz, +12 dB drive, 8x: worst alias {alias:.1} dBc");
    assert!(alias <= -90.0, "worst alias {alias:.1} dBc, want <= -90");
}

#[test]
fn off_is_a_wire() {
    let input = sine(300.0, 1.4, 9_600);
    let out = run(&ClipperConfig::default(), &input);
    assert!(out.iter().zip(&input).all(|(a, b)| a.to_bits() == b.to_bits()));
}

#[test]
fn switching_off_fades_back_to_a_wire() {
    let input = sine(300.0, 1.4, 48_000);
    let mut c = Clipper::new(SR);
    let mut l = input.clone();
    let mut r = input.clone();
    for (b, start) in (0..l.len()).step_by(BLOCK).enumerate() {
        let end = (start + BLOCK).min(l.len());
        let on = b < 40;
        c.process_stereo(&mut l[start..end], &mut r[start..end], &ClipperConfig { enabled: on, ..cfg(6.0, 0.0) });
    }
    assert!(peak(&l[20 * BLOCK..40 * BLOCK]) < 0.52, "was not clipping while on");
    let from = 45 * BLOCK;
    assert!(l[from..].iter().zip(&input[from..]).all(|(a, b)| a.to_bits() == b.to_bits()));
    // No step anywhere in the transition.
    let max_step = l.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(max_step < 0.1, "a step of {max_step} in the output");
}

#[test]
fn the_clipper_adds_no_latency() {
    assert_eq!(Clipper::new(SR).latency(), 0);
    let latency = |on: bool| {
        let mut p = ResonanceMastering::new();
        p.params().clipper.on.set_value(on);
        p.params().clipper.drive.set_value(9.0);
        p.initialize(SR, 512);
        p.latency_samples()
    };
    assert_eq!(latency(false), latency(true));
}
