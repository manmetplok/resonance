//! Per-band width in the imager (`img_b{n}_width`, warmth-width-depth.md
//! §6.3), riding on the multiband's crossover bands. Unity everywhere is
//! the old path bit-for-bit; 0 on the low band folds the low band to
//! mono without touching the rest; width never moves the mono sum; and
//! the latency does not change.

use resonance_mastering::params::MasteringParams;
use resonance_mastering::stages::multiband::{Multiband, MultibandConfig, NUM_BANDS};
use resonance_mastering::ResonanceMastering;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
const BLOCK: usize = 512;
const TAU: f32 = std::f32::consts::TAU;

fn sine(freq: f32, n: usize) -> f32 {
    (n as f32 / SR * freq * TAU).sin()
}

/// A 30 Hz tone that is pure side (antiphase), a 5 kHz tone that is
/// pure side, and a 400 Hz tone that is pure mid.
fn wide_input(len: usize) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0; len];
    let mut r = vec![0.0; len];
    for n in 0..len {
        let (lo, hi, mid) = (0.3 * sine(30.0, n), 0.2 * sine(5000.0, n), 0.3 * sine(400.0, n));
        l[n] = mid + lo + hi;
        r[n] = mid - lo - hi;
    }
    (l, r)
}

fn run_stage(
    mb: &mut Multiband,
    l: &[f32],
    r: &[f32],
    cfg: &MultibandConfig,
    width_at: impl Fn(usize) -> [f32; NUM_BANDS],
) -> (Vec<f32>, Vec<f32>) {
    let (mut ol, mut or) = (l.to_vec(), r.to_vec());
    for (b, start) in (0..l.len()).step_by(BLOCK).enumerate() {
        let end = (start + BLOCK).min(l.len());
        mb.process_stereo_with_width(&mut ol[start..end], &mut or[start..end], cfg, &width_at(b));
    }
    (ol, or)
}

/// Amplitude of `freq` in `x` (single-bin DFT; the tones sit on whole
/// cycles over the analysed span often enough for this to be exact to
/// well under 0.1 dB).
fn tone_amp(x: &[f32], freq: f32) -> f64 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, &v) in x.iter().enumerate() {
        let ph = (n as f64) * (freq as f64) * std::f64::consts::TAU / SR as f64;
        re += v as f64 * ph.cos();
        im += v as f64 * ph.sin();
    }
    2.0 * (re * re + im * im).sqrt() / x.len() as f64
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-12).log10()
}

#[test]
fn unity_width_is_bit_exact_on_the_stage() {
    let lat = Multiband::latency_for(SR);
    let (l, r) = wide_input(lat + 24_000);
    for enabled in [false, true] {
        let cfg = MultibandConfig {
            enabled,
            ..MultibandConfig::default()
        };
        let mut a = Multiband::new(SR, BLOCK);
        let mut b = Multiband::new(SR, BLOCK);
        let (al, ar) = run_stage(&mut a, &l, &r, &cfg, |_| [1.0; NUM_BANDS]);
        let (mut bl, mut br) = (l.clone(), r.clone());
        for start in (0..l.len()).step_by(BLOCK) {
            let end = (start + BLOCK).min(l.len());
            b.process_stereo(&mut bl[start..end], &mut br[start..end], &cfg);
        }
        assert!(al.iter().zip(&bl).all(|(x, y)| x.to_bits() == y.to_bits()));
        assert!(ar.iter().zip(&br).all(|(x, y)| x.to_bits() == y.to_bits()));
    }
}

/// Widths moved and then set back to unity: once the ramp is done, the
/// stage is on the old path again, bit-for-bit.
#[test]
fn returning_to_unity_restores_the_exact_path() {
    let lat = Multiband::latency_for(SR);
    let (l, r) = wide_input(lat + 48_000);
    for enabled in [false, true] {
        let cfg = MultibandConfig {
            enabled,
            ..MultibandConfig::default()
        };
        let mut moved = Multiband::new(SR, BLOCK);
        let mut steady = Multiband::new(SR, BLOCK);
        let (ml, mr) = run_stage(&mut moved, &l, &r, &cfg, |b| {
            if (5..20).contains(&b) {
                [0.5, 1.0, 1.8, 1.0]
            } else {
                [1.0; NUM_BANDS]
            }
        });
        let (sl, sr) = run_stage(&mut steady, &l, &r, &cfg, |_| [1.0; NUM_BANDS]);
        assert!((5 * BLOCK..20 * BLOCK).any(|i| ml[i] != sl[i]), "width never reached the output");
        let from = 22 * BLOCK + lat;
        for i in from..l.len() {
            assert_eq!(ml[i].to_bits(), sl[i].to_bits(), "enabled={enabled}: left differs at {i}");
            assert_eq!(mr[i].to_bits(), sr[i].to_bits(), "enabled={enabled}: right differs at {i}");
        }
    }
}

#[test]
fn zero_width_on_the_low_band_makes_it_mono() {
    let lat = Multiband::latency_for(SR);
    let len = lat + 48_000;
    let (l, r) = wide_input(len);
    let mut mb = Multiband::new(SR, BLOCK);
    // Multiband compression off: the width still gets the split.
    let (ol, or) = run_stage(&mut mb, &l, &r, &MultibandConfig::default(), |_| [0.0, 1.0, 1.0, 1.0]);

    let from = lat + 12_000;
    let side: Vec<f32> = (from..len).map(|i| 0.5 * (ol[i] - or[i])).collect();
    let mid: Vec<f32> = (from..len).map(|i| 0.5 * (ol[i] + or[i])).collect();
    let lo = db(tone_amp(&side, 30.0) / 0.3);
    let hi = db(tone_amp(&side, 5000.0) / 0.2);
    let m = db(tone_amp(&mid, 400.0) / 0.3);
    // The 30 Hz side sits in the low band (LR4 at 120 Hz leaks ~-48 dB
    // of it into the next band, which keeps its width).
    assert!(lo < -40.0, "30 Hz side only fell to {lo:.1} dB");
    assert!(hi.abs() < 0.05, "5 kHz side moved {hi:.3} dB");
    assert!(m.abs() < 0.05, "400 Hz mid moved {m:.3} dB");
}

#[test]
fn width_never_moves_the_mono_sum() {
    let lat = Multiband::latency_for(SR);
    let len = lat + 24_000;
    let (l, r) = wide_input(len);
    let mut mb = Multiband::new(SR, BLOCK);
    let (ol, or) = run_stage(&mut mb, &l, &r, &MultibandConfig::default(), |b| {
        let t = (b % 17) as f32 / 16.0;
        [2.0 * t, 1.5, 0.3, 2.0 - 2.0 * t]
    });
    let mut worst = 0.0f32;
    for i in lat + 8192..len {
        let want = 0.5 * (l[i - lat] + r[i - lat]);
        worst = worst.max((0.5 * (ol[i] + or[i]) - want).abs());
    }
    assert!(worst < 2e-5, "mono sum moved by {worst:.3e}");
}

fn render(setup: impl Fn(&MasteringParams), blocks: usize) -> (Vec<f32>, u32) {
    let mut plugin = ResonanceMastering::new();
    setup(plugin.params());
    plugin.initialize(SR, BLOCK as u32);
    let latency = plugin.latency_samples();
    let (l, r) = wide_input(blocks * BLOCK);
    let mut out = Vec::new();
    for b in 0..blocks {
        let mut bl = l[b * BLOCK..(b + 1) * BLOCK].to_vec();
        let mut br = r[b * BLOCK..(b + 1) * BLOCK].to_vec();
        let mut outs = [OutputBuffer {
            left: &mut bl,
            right: &mut br,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, BLOCK, &mut ev, None);
        out.extend_from_slice(&bl);
        out.extend_from_slice(&br);
    }
    assert_eq!(plugin.latency_samples(), latency, "latency changed while running");
    (out, latency)
}

/// With the imager on and every band width left at (or set back to)
/// 1.0, the plugin renders exactly what it rendered without the band
/// widths.
#[test]
fn unity_band_widths_are_bit_exact_in_the_plugin() {
    let base = |p: &MasteringParams| {
        p.imager.on.set_value(true);
        p.imager.width.set_value(1.4);
    };
    let (a, la) = render(base, 80);
    let (b, lb) = render(
        |p| {
            base(p);
            for w in &p.imager.band_width {
                w.set_value(1.0);
            }
        },
        80,
    );
    assert_eq!(la, lb);
    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
    let peak = a.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak > 0.1, "rendered silence");
}

/// Band widths set while the imager is off do nothing.
#[test]
fn band_widths_follow_the_imager_switch() {
    let (a, _) = render(|_| {}, 60);
    let (b, _) = render(
        |p| {
            p.imager.band_width[0].set_value(0.0);
            p.imager.band_width[3].set_value(2.0);
        },
        60,
    );
    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
}

#[test]
fn band_width_does_not_change_the_latency() {
    let (_, plain) = render(|_| {}, 4);
    let (_, widened) = render(
        |p| {
            p.imager.on.set_value(true);
            p.imager.band_width[0].set_value(0.0);
            p.imager.band_width[2].set_value(1.7);
        },
        4,
    );
    assert_eq!(plain, widened);
}
