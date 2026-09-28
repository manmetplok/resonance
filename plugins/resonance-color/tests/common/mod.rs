//! Signals and render helpers shared by the Color tests.

#![allow(dead_code)]

use resonance_color::dsp::Settings;
use resonance_color::params::{ColorParams, Mode};
use resonance_color::ResonanceColor;
use resonance_dsp::{OversampleFactor, SimpleRng};
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};

pub const SR: f32 = 48_000.0;
pub const BLOCK: usize = 256;

/// White noise in [-1, 1).
fn white(rng: &mut SimpleRng) -> f32 {
    let u = (rng.next_u32() >> 8) as f32 / (1u32 << 24) as f32;
    2.0 * u - 1.0
}

/// `n` samples of stereo pink noise (Paul Kellet's filter), scaled to
/// `rms_dbfs` RMS per channel; the channels are independent.
pub fn pink_noise(n: usize, rms_dbfs: f32, seed: u64) -> (Vec<f32>, Vec<f32>) {
    let mut out = [Vec::with_capacity(n), Vec::with_capacity(n)];
    for (c, ch) in out.iter_mut().enumerate() {
        let mut rng = SimpleRng::new(seed + c as u64 * 7919);
        let mut b = [0.0f32; 7];
        for _ in 0..n {
            let w = white(&mut rng);
            b[0] = 0.99886 * b[0] + w * 0.0555179;
            b[1] = 0.99332 * b[1] + w * 0.0750759;
            b[2] = 0.96900 * b[2] + w * 0.153852;
            b[3] = 0.86650 * b[3] + w * 0.3104856;
            b[4] = 0.55000 * b[4] + w * 0.5329522;
            b[5] = -0.7616 * b[5] - w * 0.0168980;
            let p = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
            b[6] = w * 0.115926;
            ch.push(p);
        }
    }
    let [mut l, mut r] = out;
    for ch in [&mut l, &mut r] {
        let rms = (ch.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
        let g = (10f64.powf(rms_dbfs as f64 / 20.0) / rms) as f32;
        ch.iter_mut().for_each(|v| *v *= g);
    }
    (l, r)
}

/// A drum-ish loop at 120 BPM: a pitch-dropping kick on every beat, a
/// noisy snare on 2 and 4, closed hats on the sixteenths — dense program
/// material, the kind a drum bus carries. Peak-normalised to
/// `peak_dbfs`; the right channel carries the hats a little louder so
/// the two channels differ.
///
/// It is dense on purpose. A sparse pattern (kick on 1 and 3 only) has
/// 400 ms blocks with no hit in them, BS.1770's relative gate drops
/// those, and any balance change between kick and snare — which a head
/// bump is — then moves *which* blocks are gated: the integrated reading
/// shifts by a few tenths of an LU with no change in the matched power.
pub fn drum_loop(n: usize, peak_dbfs: f32, seed: u64) -> (Vec<f32>, Vec<f32>) {
    let mut rng = SimpleRng::new(seed);
    let beat = (SR * 0.5) as usize;
    let sixteenth = beat / 4;
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];
    let mut hat_lp = 0.0f32;
    for i in 0..n {
        let in_bar = i % (4 * beat);
        let beat_idx = in_bar / beat;
        let tb = (in_bar % beat) as f32 / SR;
        let ts = (i % sixteenth) as f32 / SR;
        // 110 → 45 Hz sweep (the phase integral of 45 + 65·e^(−30t)),
        // 180 ms decay.
        let phase =
            std::f32::consts::TAU * (45.0 * tb + 65.0 / 30.0 * (1.0 - (-tb * 30.0).exp()));
        let kick = (-tb / 0.18).exp() * phase.sin();
        let mut snare = 0.0;
        if beat_idx % 2 == 1 {
            let noise = white(&mut rng);
            snare = (-tb / 0.08).exp()
                * (0.6 * noise + 0.5 * (std::f32::consts::TAU * 190.0 * tb).sin());
        }
        let w = white(&mut rng);
        // Crude high-pass: noise minus its lowpass.
        hat_lp += 0.3 * (w - hat_lp);
        let hat = (-ts / 0.02).exp() * (w - hat_lp) * 0.3;
        l[i] = kick + snare + 0.8 * hat;
        r[i] = kick + snare + hat;
    }
    let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
    let g = 10f32.powf(peak_dbfs / 20.0) / peak;
    l.iter_mut().chain(r.iter_mut()).for_each(|v| *v *= g);
    (l, r)
}

/// Apply `s` to the plugin's params (so the real param → settings path
/// is exercised, not just the DSP).
pub fn apply_settings(params: &ColorParams, s: &Settings) {
    params.mode.set_plain(s.mode as i32 as f64);
    params.drive.set_value(s.drive);
    params.bias.set_value(s.bias);
    params.response.set_value(s.response_db);
    params.tone.set_value(s.tone_db);
    params.mix.set_value(s.mix);
    params.auto_gain.set_value(s.auto_gain);
    params.output.set_value(s.output_db);
    params.oversample.set_plain(s.oversample as i32 as f64);
    let speed = resonance_color::params::SPEED_IPS
        .iter()
        .position(|&v| v == s.speed_ips)
        .expect("a declared tape speed");
    params.speed.set_plain(speed as f64);
    params.flutter.set_value(s.flutter);
}

/// Render `(l, r)` through a fresh plugin with `s` applied, in
/// `BLOCK`-sized host blocks.
pub fn render(s: &Settings, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut plugin = ResonanceColor::new();
    apply_settings(&plugin.params, s);
    plugin.initialize(SR, BLOCK as u32);
    render_with(&mut plugin, l, r)
}

pub fn render_with(plugin: &mut ResonanceColor, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut out_l = l.to_vec();
    let mut out_r = r.to_vec();
    let mut n = 0;
    while n < l.len() {
        let frames = BLOCK.min(l.len() - n);
        let mut outs = [OutputBuffer {
            left: &mut out_l[n..n + frames],
            right: &mut out_r[n..n + frames],
        }];
        plugin.process(&mut outs, frames, &mut EventIterator::empty(), None);
        n += frames;
    }
    (out_l, out_r)
}

pub fn settings(mode: Mode) -> Settings {
    Settings {
        mode,
        ..Settings::default()
    }
}

pub fn factors() -> [OversampleFactor; 3] {
    [OversampleFactor::Off, OversampleFactor::X2, OversampleFactor::X4]
}

/// A stereo sine.
pub fn sine(n: usize, freq: f32, amp: f32) -> (Vec<f32>, Vec<f32>) {
    let l: Vec<f32> = (0..n)
        .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / SR).sin())
        .collect();
    let r = l.iter().map(|v| v * 0.8).collect();
    (l, r)
}

/// The IIR oversampler round trip of `x` at `factor` — what the dry path
/// is at 2× and 4×.
pub fn round_trip(x: &[f32], factor: OversampleFactor) -> Vec<f32> {
    let mut os = resonance_dsp::Oversampler::new();
    os.set_factor(factor);
    x.iter()
        .map(|&s| {
            let buf = os.upsample(s);
            os.downsample(&buf)
        })
        .collect()
}

/// Stereo white noise in [-amp, amp), independent channels.
pub fn white_noise(n: usize, amp: f32, seed: u64) -> (Vec<f32>, Vec<f32>) {
    let mut rng = SimpleRng::new(seed);
    let l = (0..n).map(|_| amp * white(&mut rng)).collect();
    let r = (0..n).map(|_| amp * white(&mut rng)).collect();
    (l, r)
}
