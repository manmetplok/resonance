use resonance_dsp::SimpleRng;
use resonance_mastering::stages::dither::{tpdf_sample, Dither, DitherConfig};

#[test]
fn disabled_passes_audio_unchanged() {
    let mut d = Dither::new();
    let mut l = vec![0.1, 0.2, -0.3, 0.4];
    let mut r = vec![-0.1, 0.2, 0.3, -0.4];
    let el = l.clone();
    let er = r.clone();
    d.process_stereo(&mut l, &mut r, &DitherConfig::default());
    assert_eq!(l, el);
    assert_eq!(r, er);
}

#[test]
fn enabled_dither_stays_within_two_lsb() {
    // TPDF on [-lsb, lsb] means any added noise has magnitude ≤ lsb.
    // Feed silence and verify the output never exceeds ±lsb.
    let mut d = Dither::new();
    let cfg = DitherConfig {
        enabled: true,
        target_bits: 16,
        noise_shape: false,
    };
    let lsb = 2.0_f32.powi(-15);
    let n = 8192;
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    d.process_stereo(&mut l, &mut r, &cfg);
    let peak = l
        .iter()
        .chain(r.iter())
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max);
    assert!(peak <= lsb * 1.01, "TPDF peak = {peak} vs lsb {lsb}");
}

#[test]
fn dither_magnitude_scales_with_bit_depth() {
    // 24-bit dither should be much quieter than 16-bit dither.
    let mut d16 = Dither::new();
    let mut d24 = Dither::new();
    let n = 4096;
    let mut l16 = vec![0.0_f32; n];
    let mut r16 = vec![0.0_f32; n];
    let mut l24 = vec![0.0_f32; n];
    let mut r24 = vec![0.0_f32; n];
    d16.process_stereo(
        &mut l16,
        &mut r16,
        &DitherConfig {
            enabled: true,
            target_bits: 16,
            noise_shape: false,
        },
    );
    d24.process_stereo(
        &mut l24,
        &mut r24,
        &DitherConfig {
            enabled: true,
            target_bits: 24,
            noise_shape: false,
        },
    );
    let rms16 = (l16.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
    let rms24 = (l24.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
    // 24-bit LSB is 256× smaller than 16-bit.
    let ratio = rms16 / rms24;
    assert!(
        ratio > 200.0 && ratio < 320.0,
        "16/24 dither RMS ratio = {ratio} (expected ≈ 256)"
    );
}

#[test]
fn tpdf_sample_is_triangular() {
    // Generate many samples and verify they form a rough triangular
    // distribution: most mass near zero, bounded by ±lsb.
    let mut rng = SimpleRng::new(42);
    let lsb = 1.0_f32;
    let n = 100_000usize;
    let mut near_zero = 0usize;
    let mut near_edge = 0usize;
    let mut peak = 0.0_f32;
    for _ in 0..n {
        let v = tpdf_sample(&mut rng, lsb);
        if v.abs() < lsb * 0.2 {
            near_zero += 1;
        }
        if v.abs() > lsb * 0.8 {
            near_edge += 1;
        }
        peak = peak.max(v.abs());
    }
    assert!(peak <= lsb * 1.01, "peak = {peak}");
    // Triangular distribution: density near zero is ~5× density near
    // the edge (for |v| < 0.2*lsb vs |v| > 0.8*lsb windows).
    assert!(
        near_zero > near_edge * 3,
        "near_zero = {near_zero}, near_edge = {near_edge}"
    );
}

/// Round to the `bits`-bit grid, as a DAW's integer export does.
fn quantize(x: f32, bits: i32) -> f32 {
    let scale = 2.0_f32.powi(bits - 1);
    (x * scale).round() / scale
}

/// Power of `err` below `cutoff_hz` (Hann-windowed FFT, summed bins).
fn in_band_power(err: &[f32], sr: f32, cutoff_hz: f32) -> f64 {
    use rustfft::num_complex::Complex;
    let n = err.len();
    let mut buf: Vec<Complex<f32>> = err
        .iter()
        .enumerate()
        .map(|(i, &e)| {
            let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
            Complex::new(e * w, 0.0)
        })
        .collect();
    rustfft::FftPlanner::new().plan_fft_forward(n).process(&mut buf);
    let top = (cutoff_hz / sr * n as f32) as usize;
    buf[1..top].iter().map(|c| c.norm_sqr() as f64).sum()
}

/// Export noise (output − clean input) below 4 kHz of a −60 dBFS 1 kHz
/// tone through the dither stage and a 16-bit export quantizer.
fn export_noise_in_band(noise_shape: bool) -> f64 {
    let sr = 48_000.0;
    let n = 1 << 16;
    let amp = 10f32.powf(-60.0 / 20.0);
    let clean: Vec<f32> = (0..n)
        .map(|i| amp * (std::f32::consts::TAU * 1000.0 * i as f32 / sr).sin())
        .collect();
    let (mut l, mut r) = (clean.clone(), clean.clone());
    let cfg = DitherConfig {
        enabled: true,
        target_bits: 16,
        noise_shape,
    };
    let mut d = Dither::new();
    for (bl, br) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
        d.process_stereo(bl, br, &cfg);
    }
    let err: Vec<f32> = l
        .iter()
        .zip(&clean)
        .map(|(y, x)| quantize(*y, 16) - x)
        .collect();
    in_band_power(&err, sr, 4000.0)
}

/// DSP-13: noise shaping must lower the in-band floor of the exported
/// file. Shaping the dither alone cannot — the export's own white
/// requantization error is untouched — so the stage shapes the total
/// requantization error by error feedback instead.
#[test]
fn noise_shaping_lowers_the_in_band_export_floor() {
    let flat = export_noise_in_band(false);
    let shaped = export_noise_in_band(true);
    assert!(flat > 0.0, "flat export noise must be non-zero");
    let gain_db = 10.0 * (shaped / flat).log10();
    assert!(
        gain_db < -6.0,
        "shaped in-band noise {gain_db:.2} dB relative to flat (want < -6 dB)"
    );
}

/// With shaping on, the stage quantizes to the target grid itself, so a
/// later export at the same depth is lossless.
#[test]
fn noise_shaped_output_sits_on_the_target_grid() {
    let mut d = Dither::new();
    let cfg = DitherConfig {
        enabled: true,
        target_bits: 16,
        noise_shape: true,
    };
    let mut l: Vec<f32> = (0..4096).map(|i| 0.3 * (i as f32 * 0.01).sin()).collect();
    let mut r = l.clone();
    d.process_stereo(&mut l, &mut r, &cfg);
    for &y in l.iter().chain(&r) {
        assert_eq!(quantize(y, 16), y, "{y} is off the 16-bit grid");
    }
}
