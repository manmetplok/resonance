//! Frequency-domain and timing contract of the workspace resampler
//! (LIB-01): band-limited windowed-sinc conversion, flat to 0.9 of the
//! lower Nyquist within ±0.1 dB, ≥ 80 dB rejection of aliases and
//! images, zero added delay, and a streaming form that reproduces the
//! one-shot output exactly whatever the chunking.

use resonance_common::{linear_resample_mono, linear_resample_stereo, StreamingLinearResampler};
use std::f64::consts::PI;

fn sine(freq: f64, rate: f64, frames: usize, amp: f64) -> Vec<f32> {
    (0..frames)
        .map(|i| (amp * (2.0 * PI * freq * i as f64 / rate).sin()) as f32)
        .collect()
}

fn add(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b).map(|(x, y)| x + y).collect()
}

fn interleave(l: &[f32], r: &[f32]) -> Vec<f32> {
    l.iter().zip(r).flat_map(|(a, b)| [*a, *b]).collect()
}

/// Amplitude of the `freq` component of `x` (sampled at `rate`),
/// measured over the middle of the buffer (edges skipped) with a
/// 4-term Blackman-Harris window, so a strong component elsewhere
/// cannot leak into the bin by more than ~-92 dB.
fn tone_amplitude(x: &[f32], freq: f64, rate: f64) -> f64 {
    let skip = x.len() / 8;
    let seg = &x[skip..x.len() - skip];
    let n = seg.len() as f64;
    let (a0, a1, a2, a3) = (0.35875, 0.48829, 0.14128, 0.01168);
    let (mut re, mut im, mut wsum) = (0.0f64, 0.0f64, 0.0f64);
    for (i, &s) in seg.iter().enumerate() {
        let p = 2.0 * PI * i as f64 / (n - 1.0);
        let w = a0 - a1 * p.cos() + a2 * (2.0 * p).cos() - a3 * (3.0 * p).cos();
        let ph = 2.0 * PI * freq * (i + skip) as f64 / rate;
        re += s as f64 * w * ph.cos();
        im += s as f64 * w * ph.sin();
        wsum += w;
    }
    2.0 * (re * re + im * im).sqrt() / wsum
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-30).log10()
}

fn rms_mid(x: &[f32]) -> f64 {
    let skip = x.len() / 8;
    let seg = &x[skip..x.len() - skip];
    (seg.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / seg.len() as f64).sqrt()
}

#[test]
fn upsample_passband_is_flat_to_20k() {
    // 44.1 k -> 48 k: linear interpolation loses 6.3 dB at 20 kHz.
    for freq in [1_000.0, 10_000.0, 15_000.0, 19_845.0, 20_000.0] {
        let input = sine(freq, 44_100.0, 44_100, 0.5);
        let out = linear_resample_mono(&input, 44_100.0, 48_000.0);
        let gain = db(tone_amplitude(&out, freq, 48_000.0) / 0.5);
        assert!(gain.abs() <= 0.1, "{freq} Hz: passband gain {gain:.3} dB");
    }
}

#[test]
fn downsample_passband_is_flat_to_0_9_nyquist() {
    // 96 k -> 48 k: the lower Nyquist is 24 kHz, so flat to 21.6 kHz.
    for freq in [1_000.0, 12_000.0, 20_000.0, 21_600.0] {
        let input = sine(freq, 96_000.0, 96_000, 0.5);
        let out = linear_resample_mono(&input, 96_000.0, 48_000.0);
        let gain = db(tone_amplitude(&out, freq, 48_000.0) / 0.5);
        assert!(gain.abs() <= 0.1, "{freq} Hz: passband gain {gain:.3} dB");
    }
}

#[test]
fn downsample_rejects_ultrasonic_content() {
    // A 30 kHz component at 96 k folds to 18 kHz at 48 k. It must be
    // ≥ 80 dB down; the companion 1 kHz tone proves the buffer is not
    // simply silent.
    let hf = sine(30_000.0, 96_000.0, 96_000, 1.0);
    let lf = sine(1_000.0, 96_000.0, 96_000, 1.0);
    let out = linear_resample_mono(&add(&hf, &lf), 96_000.0, 48_000.0);

    let keep = db(tone_amplitude(&out, 1_000.0, 48_000.0));
    assert!(keep.abs() <= 0.1, "1 kHz companion at {keep:.3} dB");
    let alias = db(tone_amplitude(&out, 18_000.0, 48_000.0));
    assert!(alias <= -80.0, "30 kHz aliased to 18 kHz at {alias:.1} dB");

    // Without the companion the whole output is ≥ 80 dB down, and so
    // is every other ultrasonic tone above the transition band.
    for freq in [26_500.0, 30_000.0, 36_000.0, 44_000.0, 47_000.0] {
        let out = linear_resample_mono(&sine(freq, 96_000.0, 48_000, 1.0), 96_000.0, 48_000.0);
        let level = db(rms_mid(&out) * 2f64.sqrt());
        assert!(level <= -80.0, "{freq} Hz leaks through at {level:.1} dB");
    }
}

#[test]
fn upsample_suppresses_images() {
    // 44.1 k -> 48 k of a 20 kHz tone: the spectral image at
    // 44.1 - 20 = 24.1 kHz folds to 23.9 kHz in the 48 k output.
    let out = linear_resample_mono(&sine(20_000.0, 44_100.0, 44_100, 1.0), 44_100.0, 48_000.0);
    let image = db(tone_amplitude(&out, 23_900.0, 48_000.0));
    assert!(image <= -80.0, "image at 23.9 kHz: {image:.1} dB");
}

#[test]
fn output_is_time_aligned_with_input() {
    // An impulse at input frame 1470 (44.1 k) is output time 1600 at 48 k
    // exactly (ratio 160/147). The filter is zero-phase: the peak stays
    // put and the response is symmetric around it.
    let mut x = vec![0.0f32; 4_000];
    x[1_470] = 1.0;
    let out = linear_resample_mono(&x, 44_100.0, 48_000.0);
    let peak = out
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0;
    assert_eq!(peak, 1_600);
    for k in 1..60 {
        let d = (out[1_600 - k] - out[1_600 + k]).abs();
        assert!(d < 1e-6, "asymmetric response at ±{k}: {d}");
    }

    // Downsample: frame 2000 at 96 k is frame 1000 at 48 k.
    let mut x = vec![0.0f32; 4_000];
    x[2_000] = 1.0;
    let out = linear_resample_mono(&x, 96_000.0, 48_000.0);
    let peak = out
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0;
    assert_eq!(peak, 1_000);

    // A 1 kHz sine lands on the ideal 48 k sine sample for sample —
    // no group delay, no phase shift.
    let out = linear_resample_mono(&sine(1_000.0, 44_100.0, 8_820, 0.5), 44_100.0, 48_000.0);
    let ideal = sine(1_000.0, 48_000.0, out.len(), 0.5);
    let err = out[200..out.len() - 200]
        .iter()
        .zip(&ideal[200..])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(err < 1e-4, "1 kHz sine off its ideal by {err}");
}

#[test]
fn output_length_covers_the_input_span() {
    // One output frame per output instant inside the input span:
    // ceil(len * out / in).
    for (len, from, to) in [
        (44_100usize, 44_100u32, 48_000u32),
        (1_001, 44_100, 48_000),
        (1_001, 48_000, 44_100),
        (777, 96_000, 48_000),
        (5, 8_000, 192_000),
    ] {
        let expect = (len as u64 * to as u64).div_ceil(from as u64) as usize;
        let mono = linear_resample_mono(&vec![0.1; len], from as f32, to as f32);
        assert_eq!(mono.len(), expect, "{len} @ {from} -> {to}");
        let st = linear_resample_stereo(&vec![0.1; len * 2], from as f32, to as f32);
        assert_eq!(st.len(), expect * 2, "{len} @ {from} -> {to} stereo");
    }
}

#[test]
fn dc_is_preserved_up_to_the_edges() {
    // Edges extend the first/last frame rather than zero-padding, so a
    // clip that starts or ends at a non-zero level is not faded.
    for (from, to) in [
        (44_100.0, 48_000.0),
        (96_000.0, 48_000.0),
        (48_000.0, 44_100.0),
    ] {
        let out = linear_resample_mono(&vec![0.25; 3_000], from, to);
        for (i, s) in out.iter().enumerate() {
            assert!((s - 0.25).abs() < 1e-5, "{from}->{to} frame {i}: {s}");
        }
    }
}

#[test]
fn stereo_matches_mono_per_channel() {
    let l = sine(3_000.0, 44_100.0, 5_000, 0.7);
    let r = sine(17_000.0, 44_100.0, 5_000, 0.3);
    let st = linear_resample_stereo(&interleave(&l, &r), 44_100.0, 48_000.0);
    let ml = linear_resample_mono(&l, 44_100.0, 48_000.0);
    let mr = linear_resample_mono(&r, 44_100.0, 48_000.0);
    assert_eq!(st.len(), ml.len() * 2);
    for (i, f) in st.chunks(2).enumerate() {
        assert!((f[0] - ml[i]).abs() < 1e-6 && (f[1] - mr[i]).abs() < 1e-6);
    }
}

#[test]
fn identity_rate_is_a_pass_through() {
    let x = sine(5_000.0, 48_000.0, 1_000, 0.9);
    assert_eq!(linear_resample_mono(&x, 48_000.0, 48_000.0), x);
    let st = interleave(&x, &x);
    assert_eq!(linear_resample_stereo(&st, 48_000.0, 48_000.0), st);

    let mut r = StreamingLinearResampler::new(48_000, 48_000);
    let mut out = Vec::new();
    for c in st.chunks(6) {
        r.process(c, &mut out);
    }
    r.flush(&mut out);
    assert_eq!(out, st);
}

fn stream(input: &[f32], from: u32, to: u32, sizes: &[usize]) -> Vec<f32> {
    let mut r = StreamingLinearResampler::new(from, to);
    let mut out = Vec::new();
    let mut pos = 0;
    let mut k = 0;
    while pos < input.len() {
        let frames = sizes[k % sizes.len()];
        k += 1;
        let end = (pos + frames * 2).min(input.len());
        r.process(&input[pos..end], &mut out);
        pos = end;
    }
    r.flush(&mut out);
    out
}

#[test]
fn streaming_equals_one_shot_for_any_chunking() {
    let l = add(
        &sine(440.0, 44_100.0, 9_000, 0.5),
        &sine(19_000.0, 44_100.0, 9_000, 0.3),
    );
    let r = sine(7_000.0, 44_100.0, 9_000, 0.6);
    let input = interleave(&l, &r);
    for (from, to) in [
        (44_100, 48_000),
        (48_000, 44_100),
        (96_000, 48_000),
        (22_050, 48_000),
    ] {
        let oneshot = linear_resample_stereo(&input, from as f32, to as f32);
        for sizes in [&[1usize][..], &[7], &[64], &[511], &[3, 1000, 1, 17, 256]] {
            let streamed = stream(&input, from, to, sizes);
            assert_eq!(streamed.len(), oneshot.len(), "{from}->{to} {sizes:?}");
            let err = streamed
                .iter()
                .zip(&oneshot)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(err <= 1e-6, "{from}->{to} chunks {sizes:?}: max diff {err}");
        }
    }
}

#[test]
fn streaming_flush_mid_stream_keeps_later_output_aligned() {
    // The loop-record seam flushes the resampler into the finished take
    // and keeps it running. The take gets every output instant up to the
    // seam; later output continues on the same time grid as the one-shot.
    let x = sine(1_000.0, 44_100.0, 6_000, 0.5);
    let input = interleave(&x, &x);
    let oneshot = linear_resample_stereo(&input, 44_100.0, 48_000.0);

    let seam = 2_205; // frames
    let mut r = StreamingLinearResampler::new(44_100, 48_000);
    let mut first = Vec::new();
    r.process(&input[..seam * 2], &mut first);
    r.flush(&mut first);
    assert_eq!(first.len() / 2, (seam * 48_000).div_ceil(44_100));

    let mut rest = Vec::new();
    r.process(&input[seam * 2..], &mut rest);
    r.flush(&mut rest);
    let mut all = first.clone();
    all.extend_from_slice(&rest);
    assert_eq!(all.len(), oneshot.len());
    // Past the seam's filter reach the output is the one-shot again.
    let from = first.len() + 200;
    for i in from..all.len() {
        assert!((all[i] - oneshot[i]).abs() <= 1e-6, "sample {i}");
    }
}
