use resonance_dsp::Biquad;
use resonance_metering::k_weighting::{assign_prefilter, assign_rlb};
use resonance_metering::KWeightingFilter;

#[test]
fn prefilter_matches_bs1770_48k_reference() {
    let mut bq = Biquad::identity();
    assign_prefilter(&mut bq, 48_000.0);
    // BS.1770-4 Annex 1 Table 1 values, tolerance 1e-4.
    assert!((bq.b0 - 1.535_124_9).abs() < 1e-4, "b0 = {}", bq.b0);
    assert!((bq.b1 - -2.691_696_2).abs() < 1e-4, "b1 = {}", bq.b1);
    assert!((bq.b2 - 1.198_392_8).abs() < 1e-4, "b2 = {}", bq.b2);
    assert!((bq.a1 - -1.690_659_3).abs() < 1e-4, "a1 = {}", bq.a1);
    assert!((bq.a2 - 0.732_480_8).abs() < 1e-4, "a2 = {}", bq.a2);
}

#[test]
fn rlb_matches_bs1770_48k_reference() {
    let mut bq = Biquad::identity();
    assign_rlb(&mut bq, 48_000.0);
    assert!((bq.b0 - 1.0).abs() < 1e-6);
    assert!((bq.b1 - -2.0).abs() < 1e-6);
    assert!((bq.b2 - 1.0).abs() < 1e-6);
    assert!((bq.a1 - -1.990_047_5).abs() < 1e-4, "a1 = {}", bq.a1);
    assert!((bq.a2 - 0.990_072_2).abs() < 1e-4, "a2 = {}", bq.a2);
}

#[test]
fn processes_without_nans() {
    let mut f = KWeightingFilter::new(48_000.0);
    for i in 0..4096 {
        let t = i as f32 / 48_000.0;
        let x = (t * 1000.0 * std::f32::consts::TAU).sin() * 0.5;
        let y = f.process(x);
        assert!(y.is_finite());
    }
}

#[test]
fn passes_through_mid_band_near_unity() {
    // The K-weighting curve is gentle across the midrange. Verified
    // values at 48 kHz (from both this implementation and reference
    // plots of BS.1770-4 K-weighting):
    //   1 kHz ≈ +0.6 dB, 2 kHz ≈ +1.0 dB, 5 kHz ≈ +3.0 dB.
    // The 1 kHz gain must be within a narrow band centred on +0.6.
    let sr = 48_000.0;
    let mut f = KWeightingFilter::new(sr);
    for _ in 0..4096 {
        let _ = f.process(0.0);
    }
    let freq = 1000.0_f32;
    let n = 48_000usize;
    let mut in_sq = 0.0_f64;
    let mut out_sq = 0.0_f64;
    for i in 0..n {
        let x = (i as f32 / sr * freq * std::f32::consts::TAU).sin();
        let y = f.process(x);
        if i > 4096 {
            in_sq += (x * x) as f64;
            out_sq += (y * y) as f64;
        }
    }
    let gain_db = 10.0 * (out_sq / in_sq).log10();
    assert!(
        (gain_db - 0.6).abs() < 0.3,
        "K-weighted 1 kHz gain = {gain_db} dB (expected ≈ +0.6)"
    );
}

#[test]
fn attenuates_low_frequencies() {
    // Below 100 Hz the RLB high-pass dominates and the signal is
    // attenuated by many dB. A 30 Hz sine should drop by at least 6 dB.
    let sr = 48_000.0;
    let mut f = KWeightingFilter::new(sr);
    for _ in 0..4096 {
        let _ = f.process(0.0);
    }
    let freq = 30.0_f32;
    let n = 48_000usize;
    let mut in_sq = 0.0_f64;
    let mut out_sq = 0.0_f64;
    for i in 0..n {
        let x = (i as f32 / sr * freq * std::f32::consts::TAU).sin();
        let y = f.process(x);
        if i > 4096 {
            in_sq += (x * x) as f64;
            out_sq += (y * y) as f64;
        }
    }
    let gain_db = 10.0 * (out_sq / in_sq).log10();
    assert!(
        gain_db < -6.0,
        "30 Hz K-weighted gain = {gain_db} dB (expected < -6)"
    );
}
