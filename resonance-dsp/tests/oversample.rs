//! The polyphase IIR half-band and the 1×/2×/4× oversampler: passband
//! flatness, image and alias rejection, and the group delay the design
//! trades for having no FIR latency.
//!
//! Every tone sits exactly on a DFT bin of the measurement window (10 Hz
//! bins), so a rectangular window has no leakage and a single-bin DFT reads
//! the true level down past −120 dB.

use resonance_dsp::{halfband_coefs, Halfband, OversampleFactor, Oversampler};
use std::f64::consts::TAU;

const SR: f64 = 48_000.0;

/// Magnitude of `x` at `freq` (Hz) for sample rate `sr`, as a peak
/// amplitude (a full-scale sine reads 1.0).
fn tone_level(x: &[f32], freq: f64, sr: f64) -> f64 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, &s) in x.iter().enumerate() {
        let ph = TAU * freq * n as f64 / sr;
        re += s as f64 * ph.cos();
        im -= s as f64 * ph.sin();
    }
    2.0 * (re * re + im * im).sqrt() / x.len() as f64
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-20).log10()
}

fn sine(freq: f64, sr: f64, n: usize) -> impl Iterator<Item = f32> {
    (0..n).map(move |i| (TAU * freq * i as f64 / sr).sin() as f32)
}

/// Warm-up samples discarded before measuring (base rate).
const SETTLE: usize = 4_800;
/// Measurement window (base rate): 0.1 s → 10 Hz bins.
const WINDOW: usize = 4_800;

/// Base-rate sine through up→(identity)→down at `factor`.
fn round_trip(factor: OversampleFactor, freq: f64) -> Vec<f32> {
    let mut os = Oversampler::new();
    os.set_factor(factor);
    sine(freq, SR, SETTLE + WINDOW)
        .map(|x| {
            let buf = os.upsample(x);
            os.downsample(&buf)
        })
        .skip(SETTLE)
        .collect()
}

#[test]
fn designed_coefficients_are_stable_allpasses() {
    let mut c = [0.0f32; 12];
    halfband_coefs(&mut c, 0.02);
    for (i, &k) in c.iter().enumerate() {
        assert!(k > 0.0 && k < 1.0, "coef {i} = {k} outside (0, 1)");
    }
    // The design yields an increasing sequence; branch assignment relies
    // on that ordering alternating between the two branches.
    for w in c.windows(2) {
        assert!(w[0] < w[1], "coefficients not increasing: {c:?}");
    }
}

#[test]
fn off_is_an_exact_passthrough() {
    let mut os = Oversampler::new();
    assert_eq!(os.factor(), OversampleFactor::Off);
    for x in [0.0f32, 0.3, -0.7, 1.0e-30, 12.5] {
        let buf = os.upsample(x);
        assert_eq!(buf[0].to_bits(), x.to_bits());
        assert_eq!(os.downsample(&buf).to_bits(), x.to_bits());
    }
}

#[test]
fn dc_passes_at_unity() {
    for factor in [OversampleFactor::X2, OversampleFactor::X4] {
        let mut os = Oversampler::new();
        os.set_factor(factor);
        let mut y = 0.0;
        for _ in 0..2_000 {
            let buf = os.upsample(0.5);
            for s in &buf[..os.ratio()] {
                assert!((s - 0.5).abs() < 0.5, "{factor:?}: upsampled DC ran away");
            }
            y = os.downsample(&buf);
        }
        assert!((y - 0.5).abs() < 1e-5, "{factor:?}: DC out {y}");
    }
}

#[test]
fn passband_round_trip_is_flat() {
    for factor in [OversampleFactor::X2, OversampleFactor::X4] {
        for freq in [100.0, 1_000.0, 10_000.0, 18_000.0] {
            let out = round_trip(factor, freq);
            let level = db(tone_level(&out, freq, SR));
            assert!(
                level.abs() < 0.01,
                "{factor:?}: {freq} Hz round trip {level:.4} dB"
            );
        }
        // Still within a fraction of a dB at the top of the audible band.
        let out = round_trip(factor, 20_000.0);
        let level = db(tone_level(&out, 20_000.0, SR));
        assert!(level.abs() < 0.1, "{factor:?}: 20 kHz round trip {level:.4} dB");
    }
}

/// The first stage's upsampling images: a base-rate tone `f` shows up at
/// `fs − f` at the 2× rate unless the half-band rejects it.
#[test]
fn upsampler_rejects_images() {
    let mut hb: Halfband<12> = Halfband::new(0.02);
    let hi_sr = SR * 2.0;
    for freq in [1_000.0, 10_000.0, 20_000.0] {
        hb.reset();
        let hi: Vec<f32> = sine(freq, SR, SETTLE + WINDOW)
            .flat_map(|x| hb.upsample(x))
            .skip(SETTLE * 2)
            .collect();
        let wanted = tone_level(&hi, freq, hi_sr);
        let image = tone_level(&hi, SR - freq, hi_sr);
        let rejection = db(wanted) - db(image);
        assert!(
            rejection > 100.0,
            "{freq} Hz: image at {} Hz only {rejection:.1} dB down",
            SR - freq
        );
    }
}

/// A 2×-rate tone above the base Nyquist must not survive the downsampler,
/// where it would alias to `fs − f`.
#[test]
fn downsampler_rejects_aliases() {
    let hi_sr = SR * 2.0;
    for freq in [26_000.0, 30_000.0, 40_000.0] {
        let mut hb: Halfband<12> = Halfband::new(0.02);
        let hi: Vec<f32> = sine(freq, hi_sr, (SETTLE + WINDOW) * 2).collect();
        let out: Vec<f32> = hi
            .chunks_exact(2)
            .map(|p| hb.downsample([p[0], p[1]]))
            .skip(SETTLE)
            .collect();
        let alias = tone_level(&out, SR - freq, SR);
        assert!(
            db(alias) < -100.0,
            "{freq} Hz leaked {:.1} dB into {} Hz",
            db(alias),
            SR - freq
        );
    }
}

/// At 4× only content that would fold into the base band has to go; the
/// whole chain still keeps it > 90 dB down.
#[test]
fn four_x_chain_rejects_aliases_into_the_base_band() {
    let hi_sr = SR * 4.0;
    for freq in [30_000.0, 60_000.0, 80_000.0] {
        let mut os = Oversampler::new();
        os.set_factor(OversampleFactor::X4);
        let hi: Vec<f32> = sine(freq, hi_sr, (SETTLE + WINDOW) * 4).collect();
        let out: Vec<f32> = hi
            .chunks_exact(4)
            .map(|p| os.downsample(&[p[0], p[1], p[2], p[3]]))
            .skip(SETTLE)
            .collect();
        // Where it lands after folding to the base rate.
        let folded = {
            let f = freq % SR;
            if f > SR / 2.0 {
                SR - f
            } else {
                f
            }
        };
        let alias = tone_level(&out, folded, SR);
        assert!(
            db(alias) < -90.0,
            "{freq} Hz leaked {:.1} dB into {folded} Hz",
            db(alias)
        );
    }
}

/// The IIR design has no fixed latency, but it does have a small group
/// delay at low frequencies. Pin how small, in base-rate samples, from the
/// phase of a 200 Hz tone — this is the figure the wavetable synth's docs
/// quote when explaining why it reports no latency. Measured 2026-09-27:
/// 4.04 samples at 2×, 5.51 at 4× (≈ 0.1 ms at 48 kHz).
#[test]
fn low_frequency_group_delay_is_a_few_samples() {
    for (factor, bound) in [(OversampleFactor::X2, 5.0), (OversampleFactor::X4, 6.0)] {
        let delay = group_delay_samples(factor, 200.0);
        assert!(
            delay > 0.0 && delay < bound,
            "{factor:?}: round-trip delay {delay:.2} samples"
        );
    }
}

fn phase(x: &[f32], freq: f64) -> f64 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, &s) in x.iter().enumerate() {
        let ph = TAU * freq * n as f64 / SR;
        re += s as f64 * ph.cos();
        im -= s as f64 * ph.sin();
    }
    im.atan2(re)
}

/// Phase delay at `freq` in base-rate samples. For a smooth low-frequency
/// response this equals the group delay to well within a sample.
fn group_delay_samples(factor: OversampleFactor, freq: f64) -> f64 {
    let dry: Vec<f32> = sine(freq, SR, SETTLE + WINDOW).skip(SETTLE).collect();
    let wet = round_trip(factor, freq);
    let mut d = phase(&dry, freq) - phase(&wet, freq);
    while d < 0.0 {
        d += TAU;
    }
    d / TAU * SR / freq
}
