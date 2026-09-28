//! The saturator's `sat_mode` voicings (warmth-width-depth.md §6.1,
//! §6.3). The default, Blend, is the original stage bit-for-bit; every
//! other mode has a pinned harmonic signature; none moves the latency.

use resonance_mastering::params::MasteringParams;
use resonance_mastering::stages::saturator::{SatMode, Saturator, SaturatorConfig};
use resonance_mastering::ResonanceMastering;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

const SR: f32 = 48_000.0;
const BLOCK: usize = 480;
const TAU: f64 = std::f64::consts::TAU;

const MODES: [SatMode; 6] = [
    SatMode::Tube,
    SatMode::Tape,
    SatMode::Transformer,
    SatMode::Console,
    SatMode::Warm,
    SatMode::Inflator,
];

fn sine(freq: f64, amp: f64, len: usize) -> Vec<f32> {
    (0..len)
        .map(|n| (amp * (TAU * freq * n as f64 / SR as f64).sin()) as f32)
        .collect()
}

fn cfg(mode: SatMode, drive_db: f32) -> SaturatorConfig {
    SaturatorConfig {
        enabled: true,
        drive_db,
        mix: 1.0,
        mode,
        ..SaturatorConfig::default()
    }
}

fn run(cfg: &SaturatorConfig, input: &[f32]) -> Vec<f32> {
    let mut s = Saturator::new(SR);
    let mut l = input.to_vec();
    let mut r = input.to_vec();
    for start in (0..l.len()).step_by(BLOCK) {
        let end = (start + BLOCK).min(l.len());
        s.process_stereo(&mut l[start..end], &mut r[start..end], cfg);
    }
    assert_eq!(l, r, "identical channels must saturate identically");
    l
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-15).log10()
}

/// Magnitudes of one second of signal (1 Hz bins, no leakage for tones
/// on whole Hz).
fn spectrum(x: &[f32]) -> Vec<f64> {
    assert_eq!(x.len(), SR as usize);
    let mut buf: Vec<Complex<f64>> = x.iter().map(|&v| Complex::new(v as f64, 0.0)).collect();
    FftPlanner::new().plan_fft_forward(x.len()).process(&mut buf);
    buf[..x.len() / 2].iter().map(|c| c.norm()).collect()
}

/// H2..H5 of `f0` in dBc.
fn harmonics(x: &[f32], f0: usize) -> [f64; 4] {
    let s = spectrum(x);
    std::array::from_fn(|k| db(s[(k + 2) * f0] / s[f0]))
}

/// Total harmonic distortion (H2..H9) in dB relative to the fundamental.
fn thd_db(x: &[f32], f0: usize) -> f64 {
    let s = spectrum(x);
    let p: f64 = (2..10).map(|k| s[k * f0].powi(2)).sum();
    db(p.sqrt() / s[f0])
}

/// A −6 dBFS 1 kHz tone at +6 dB drive, one settled second.
fn signature(mode: SatMode) -> [f64; 4] {
    let out = run(&cfg(mode, 6.0), &sine(1000.0, 0.5, 2 * SR as usize));
    harmonics(&out[SR as usize..], 1000)
}

/// Pinned H2..H5 (dBc) per mode, measured on the canonical machine.
/// `None` for an order the voicing must not produce at all.
fn pinned(mode: SatMode) -> [Option<f64>; 4] {
    match mode {
        SatMode::Tube => [Some(-15.5), Some(-28.8), Some(-34.8), Some(-79.2)],
        SatMode::Tape => [Some(-25.6), Some(-24.0), Some(-42.9), Some(-46.3)],
        SatMode::Transformer => [Some(-29.1), Some(-23.7), Some(-46.2), Some(-45.5)],
        SatMode::Console => [None, Some(-39.6), None, Some(-89.7)],
        SatMode::Warm => [Some(-19.7), None, Some(-47.2), None],
        SatMode::Inflator => [None, Some(-19.3), None, Some(-67.2)],
        SatMode::Blend => unreachable!(),
    }
}

#[test]
fn per_mode_harmonic_signatures_are_pinned() {
    let mut report = String::new();
    let mut failures = Vec::new();
    for mode in MODES {
        let got = signature(mode);
        report += &format!(
            "{mode:?}: H2 {:.1} H3 {:.1} H4 {:.1} H5 {:.1}\n",
            got[0], got[1], got[2], got[3]
        );
        for (k, (g, want)) in got.iter().zip(pinned(mode)).enumerate() {
            let ok = match want {
                // An order the curve cannot make: only rounding.
                None => *g < -110.0,
                Some(w) => (g - w).abs() <= 0.5,
            };
            if !ok {
                failures.push(format!("{mode:?} H{}: got {g:.1}, want {want:?}", k + 2));
            }
        }
    }
    eprint!("{report}");
    assert!(failures.is_empty(), "{}\n{report}", failures.join("\n"));
}

#[test]
fn the_voicings_differ_the_way_they_are_named() {
    let tube = signature(SatMode::Tube);
    assert!(tube[0] > tube[1] + 6.0, "Tube must be H2-dominant: {tube:?}");
    let warm = signature(SatMode::Warm);
    assert!(warm[0] > -40.0 && warm[1] < -110.0, "Warm must be even-only: {warm:?}");
    for odd in [SatMode::Console, SatMode::Inflator] {
        let h = signature(odd);
        assert!(h[0] < -110.0 && h[1] > -60.0, "{odd:?} must be odd-only: {h:?}");
    }
    // Transformer: bass saturates first.
    let lo = run(&cfg(SatMode::Transformer, 9.0), &sine(50.0, 0.5, 2 * SR as usize));
    let hi = run(&cfg(SatMode::Transformer, 9.0), &sine(2000.0, 0.5, 2 * SR as usize));
    let (tl, th) = (thd_db(&lo[SR as usize..], 50), thd_db(&hi[SR as usize..], 2000));
    assert!(tl > th + 6.0, "Transformer THD 50 Hz {tl:.1} dB vs 2 kHz {th:.1} dB");
}

#[test]
fn even_voicings_leave_no_dc() {
    for mode in [SatMode::Tube, SatMode::Tape, SatMode::Warm, SatMode::Transformer] {
        let out = run(&cfg(mode, 12.0), &sine(200.0, 0.7, 2 * SR as usize));
        let tail = &out[SR as usize..];
        let mean = tail.iter().map(|&v| v as f64).sum::<f64>() / tail.len() as f64;
        assert!(mean.abs() < 1e-3, "{mode:?} leaves DC {mean:.2e}");
    }
}

#[test]
fn every_mode_is_stable_at_full_drive_on_full_scale_noise() {
    let mut seed = 1u32;
    let noise: Vec<f32> = (0..48_000)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1 << 23) as f32 - 1.0
        })
        .collect();
    for mode in MODES {
        for curve in [-0.5f32, 0.0, 0.5] {
            let c = SaturatorConfig {
                curve,
                ..cfg(mode, 18.0)
            };
            let out = run(&c, &noise);
            assert!(out.iter().all(|v| v.is_finite()), "{mode:?} produced non-finite output");
            let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak < 4.0, "{mode:?} (curve {curve}) peaked at {peak}");
        }
    }
}

#[test]
fn mix_zero_is_the_dry_signal() {
    for mode in MODES {
        let input = sine(1000.0, 0.5, 2 * SR as usize);
        let out = run(
            &SaturatorConfig {
                mix: 0.0,
                ..cfg(mode, 12.0)
            },
            &input,
        );
        // The dry path runs through the same latency-free up/down pair
        // as the wet one, so it is the input up to a small phase shift.
        let s_in = spectrum(&input[SR as usize..]);
        let s_out = spectrum(&out[SR as usize..]);
        let gain = db(s_out[1000] / s_in[1000]);
        assert!(gain.abs() < 0.01, "{mode:?}: dry path gain {gain:.3} dB");
        assert!(harmonics(&out[SR as usize..], 1000).iter().all(|&h| h < -110.0));
    }
}

fn render(setup: impl Fn(&MasteringParams), blocks: usize) -> (Vec<f32>, u32) {
    let mut plugin = ResonanceMastering::new();
    setup(plugin.params());
    plugin.initialize(SR, 512);
    let latency = plugin.latency_samples();
    let input = sine(220.0, 0.8, blocks * 512);
    let mut out = Vec::new();
    for b in 0..blocks {
        let mut l = input[b * 512..(b + 1) * 512].to_vec();
        let mut r = l.clone();
        let mut outs = [OutputBuffer {
            left: &mut l,
            right: &mut r,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, 512, &mut ev, None);
        out.extend_from_slice(&l);
    }
    assert_eq!(plugin.latency_samples(), latency);
    (out, latency)
}

/// The default mode is the original stage: with Blend explicitly set,
/// and an Inflator curve that Blend ignores, the render is bit-identical.
#[test]
fn the_default_mode_is_bit_identical() {
    let base = |p: &MasteringParams| {
        p.saturator.on.set_value(true);
        p.saturator.drive.set_value(9.0);
        p.saturator.character.set_value(0.6);
        p.saturator.shaper.set_value(1);
    };
    let (a, _) = render(base, 60);
    let (b, _) = render(
        |p| {
            base(p);
            p.saturator.mode.set_value(SatMode::Blend.to_index());
            p.saturator.curve.set_value(0.4);
        },
        60,
    );
    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
    assert_eq!(MasteringParams::default().saturator.mode.value(), SatMode::Blend.to_index());
}

#[test]
fn no_mode_changes_the_latency() {
    let (_, plain) = render(|_| {}, 2);
    for mode in MODES {
        let (_, l) = render(
            |p| {
                p.saturator.on.set_value(true);
                p.saturator.mode.set_value(mode.to_index());
            },
            2,
        );
        assert_eq!(l, plain, "{mode:?}");
    }
}

/// Phase delay of `out` against `input` at `freq`, in samples, from the
/// last half of the render.
fn phase_delay(input: &[f32], out: &[f32], freq: f64) -> f64 {
    let phase = |x: &[f32]| {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (n, &s) in x.iter().enumerate() {
            let ph = TAU * freq * n as f64 / SR as f64;
            re += s as f64 * ph.cos();
            im -= s as f64 * ph.sin();
        }
        im.atan2(re)
    };
    let half = input.len() / 2;
    let mut d = phase(&input[half..]) - phase(&out[half..]);
    while d < 0.0 {
        d += TAU;
    }
    d / TAU * SR as f64 / freq
}

/// The group delay the module and chain docs quote: a non-Blend mode
/// reports no latency, but its 4x IIR pair delays the signal by ~5.5
/// samples below a few kHz (review finding M3). Console at 0 dB drive is
/// the identity curve, so what is left is the pair.
#[test]
fn the_mode_group_delay_is_the_documented_one() {
    for freq in [200.0, 1000.0, 3000.0] {
        let input = sine(freq, 0.25, 96_000);
        let out = run(&cfg(SatMode::Console, 0.0), &input);
        let d = phase_delay(&input, &out, freq);
        assert!((5.3..5.8).contains(&d), "{freq} Hz: {d:.2} samples");
    }
}

/// The enable crossfade of a mode combs for its 10 ms (the wet path is
/// ~5.5 samples late) but must never step: toggled on and off on a tone,
/// no output sample moves further than the tone itself can.
#[test]
fn mode_enable_fades_do_not_step() {
    let input = sine(1000.0, 0.25, 48_000);
    let tone_step = input.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    for mode in MODES {
        let mut s = Saturator::new(SR);
        let (mut l, mut r) = (input.clone(), input.clone());
        for (b, start) in (0..l.len()).step_by(BLOCK).enumerate() {
            let end = (start + BLOCK).min(l.len());
            let cfg = SaturatorConfig {
                enabled: (20..60).contains(&b),
                ..cfg(mode, 0.0)
            };
            s.process_stereo(&mut l[start..end], &mut r[start..end], &cfg);
        }
        let slope = |x: &[f32]| x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        // A mode's own gain (the Inflator's ~1.5x at 0 dB drive) scales
        // the wet tone's slope, so the bound is the steeper of the two.
        let wet_step = slope(&l[40 * BLOCK..50 * BLOCK]);
        let max_step = slope(&l);
        assert!(
            max_step <= tone_step.max(wet_step) * 1.05,
            "{mode:?}: step {max_step} vs the tone's {tone_step} (wet {wet_step})"
        );
    }
}
