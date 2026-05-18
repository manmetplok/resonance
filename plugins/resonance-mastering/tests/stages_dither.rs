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
