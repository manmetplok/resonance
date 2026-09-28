//! Render and measurement helpers shared by the stereo plugin's tests.

#![allow(dead_code)]

use resonance_dsp::Biquad;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use resonance_stereo::params::StereoParams;
use resonance_stereo::ResonanceStereo;

pub const SR: f32 = 48_000.0;
pub const MAX_BLOCK: usize = 512;

/// Deterministic white noise in [-amp, amp), identical however it is
/// chopped into blocks.
pub fn noise(n: usize, amp: f32, seed: u64) -> Vec<f32> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let u = (s >> 40) as f32 / (1u64 << 24) as f32;
            amp * (2.0 * u - 1.0)
        })
        .collect()
}

pub fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| amp * (std::f64::consts::TAU * freq as f64 * i as f64 / SR as f64).sin() as f32)
        .collect()
}

/// A plugin configured by `setup` (before `initialize`, so smoothers
/// start settled on the configured values), rendered over `(l, r)` in
/// blocks cycling through `blocks`.
pub fn render_with(
    setup: impl Fn(&StereoParams),
    l: &[f32],
    r: &[f32],
    blocks: &[usize],
) -> (Vec<f32>, Vec<f32>, ResonanceStereo) {
    let mut plugin = ResonanceStereo::new();
    setup(&plugin.params);
    plugin.initialize(SR, MAX_BLOCK as u32);
    plugin.reset();
    let (mut out_l, mut out_r) = (l.to_vec(), r.to_vec());
    let mut pos = 0;
    let mut k = 0;
    while pos < l.len() {
        let frames = blocks[k % blocks.len()].min(l.len() - pos);
        k += 1;
        let mut outs = [OutputBuffer {
            left: &mut out_l[pos..pos + frames],
            right: &mut out_r[pos..pos + frames],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, frames, &mut ev, None);
        pos += frames;
    }
    (out_l, out_r, plugin)
}

pub fn render(setup: impl Fn(&StereoParams), l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let (a, b, _) = render_with(setup, l, r, &[256]);
    (a, b)
}

/// Pearson correlation of two equal-length signals.
pub fn correlation(a: &[f32], b: &[f32]) -> f64 {
    let (mut ab, mut aa, mut bb) = (0.0f64, 0.0f64, 0.0f64);
    for (&x, &y) in a.iter().zip(b) {
        ab += x as f64 * y as f64;
        aa += x as f64 * x as f64;
        bb += y as f64 * y as f64;
    }
    ab / (aa * bb).sqrt().max(1e-30)
}

pub fn energy(x: &[f32]) -> f64 {
    x.iter().map(|&v| v as f64 * v as f64).sum()
}

/// Side-to-mid power ratio in dB.
pub fn side_mid_db(l: &[f32], r: &[f32]) -> f64 {
    let (mut m, mut s) = (0.0f64, 0.0f64);
    for (&a, &b) in l.iter().zip(r) {
        let mm = 0.5 * (a as f64 + b as f64);
        let ss = 0.5 * (a as f64 - b as f64);
        m += mm * mm;
        s += ss * ss;
    }
    10.0 * (s / m.max(1e-30)).log10()
}

/// `x` through `order`/2 cascaded Butterworth sections (a steep
/// measurement filter), low-pass or high-pass at `freq`.
pub fn band(x: &[f32], freq: f32, sections: usize, low_pass: bool) -> Vec<f32> {
    let mut fs: Vec<Biquad> = (0..sections)
        .map(|_| {
            let mut b = Biquad::identity();
            if low_pass {
                b.set_low_pass(SR, freq, std::f32::consts::FRAC_1_SQRT_2);
            } else {
                b.set_high_pass(SR, freq, std::f32::consts::FRAC_1_SQRT_2);
            }
            b
        })
        .collect();
    x.iter()
        .map(|&v| fs.iter_mut().fold(v, |acc, f| f.process(acc)))
        .collect()
}
