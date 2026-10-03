//! DSP2-01: an IR resampled to the session rate must keep its level.
//!
//! The same physical impulse response captured at two rates has the same
//! frequency response, so its taps are `h(n / fs) / fs`: twice the rate,
//! half the tap height, twice the taps. The shared resampler keeps unit
//! DC gain per sample, so without a rescale on the IR path a 96 kHz file
//! loaded at 48 kHz plays 6 dB quiet. Each test loads a band-limited IR
//! natively and resampled and compares the 1 kHz gain.

use resonance_ir::ir_loader::load_ir_from_bytes;

const TAU: f64 = std::f64::consts::TAU;

/// A smooth, band-limited "cab": two damped modes well below 20 kHz with
/// a raised-cosine onset, so every rate represents it without aliasing.
fn continuous_ir(t: f64) -> f64 {
    let onset = if t < 0.0005 {
        0.5 - 0.5 * (std::f64::consts::PI * t / 0.0005).cos()
    } else {
        1.0
    };
    onset
        * ((-300.0 * t).exp() * (TAU * 900.0 * t).sin()
            + 0.6 * (-500.0 * t).exp() * (TAU * 2600.0 * t).sin())
}

/// The IR sampled at `fs`, scaled so the 16-bit file does not clip.
fn sampled(fs: u32) -> Vec<f32> {
    let n = (fs as f64 * 0.04) as usize;
    (0..n)
        .map(|i| (continuous_ir(i as f64 / fs as f64) * 4000.0 / fs as f64) as f32)
        .collect()
}

fn encode_mono_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for &v in samples {
        let s = (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

/// Gain of the FIR `h` at `hz` when run at `fs`, in dB.
fn gain_db(h: &[f32], hz: f64, fs: f64) -> f64 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, &v) in h.iter().enumerate() {
        let w = TAU * hz * n as f64 / fs;
        re += v as f64 * w.cos();
        im -= v as f64 * w.sin();
    }
    20.0 * (re * re + im * im).sqrt().log10()
}

fn loaded_gain(file_rate: u32, session_rate: u32) -> f64 {
    let wav = encode_mono_wav(&sampled(file_rate), file_rate);
    let ir = load_ir_from_bytes(&wav, session_rate as f32).expect("decode");
    gain_db(&ir.left, 1000.0, session_rate as f64)
}

fn assert_matches_native(file_rate: u32, session_rate: u32) {
    let native = loaded_gain(session_rate, session_rate);
    let resampled = loaded_gain(file_rate, session_rate);
    assert!(
        (native - resampled).abs() < 0.1,
        "{file_rate} Hz IR at {session_rate} Hz: 1 kHz gain {resampled:.3} dB, \
         native {native:.3} dB"
    );
}

#[test]
fn a_96k_ir_at_48k_keeps_its_level() {
    assert_matches_native(96_000, 48_000);
}

#[test]
fn a_44k1_ir_at_48k_keeps_its_level() {
    assert_matches_native(44_100, 48_000);
}

#[test]
fn a_48k_ir_at_96k_keeps_its_level() {
    assert_matches_native(48_000, 96_000);
}
