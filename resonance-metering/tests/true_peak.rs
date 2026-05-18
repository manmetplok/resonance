//! ITU-R BS.1770-4 Annex 2 true-peak validation.

mod common;

use resonance_metering::TruePeakMeter;

const SR: f32 = 48_000.0;
const TAU: f32 = std::f32::consts::TAU;

#[test]
fn detects_inter_sample_peak_above_discrete_samples() {
    // A full-scale 16 kHz cosine (fs/3) with a +π/6 phase offset lands
    // the discrete samples at max |x| = 0.866 (never the true peak).
    // The oversampler should recover a peak substantially closer to 1.0.
    let freq = 16_000.0_f32;
    let phase = std::f32::consts::PI / 6.0;
    let n = 8192;
    let mut l = vec![0.0_f32; n];
    for (i, v) in l.iter_mut().enumerate() {
        let t = i as f32 / SR;
        *v = (phase + TAU * freq * t).cos();
    }
    let r = l.clone();
    let discrete_peak = l.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);

    let mut m = TruePeakMeter::new();
    // Warm up the filter so the startup transient doesn't dominate.
    m.push_stereo(&l[..1024], &r[..1024]);
    m.reset_peak();
    m.push_stereo(&l[1024..], &r[1024..]);

    let tp = m.peak_linear();
    assert!(
        tp > discrete_peak + 0.03,
        "true peak {tp} should exceed discrete peak {discrete_peak} by at least 0.03"
    );
    // And land close to unity (0 dBTP).
    let dbtp = m.peak_dbtp();
    assert!(
        dbtp > -1.0,
        "inter-sample true peak = {dbtp} dBTP (expected > -1.0)"
    );
}

#[test]
fn true_peak_never_below_discrete_for_random_input() {
    // Fundamental correctness: for any input, the detector's held peak
    // must be ≥ max(|discrete samples|). Uses a deterministic LCG so
    // the test is reproducible.
    let n = 4096;
    let mut rng: u32 = 1_234_567_890;
    let mut l = vec![0.0_f32; n];
    for s in l.iter_mut() {
        rng = rng.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        *s = (((rng >> 16) & 0x7FFF) as f32 / 16_384.0) - 1.0;
    }
    let r = l.clone();
    let discrete = l.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    let mut m = TruePeakMeter::new();
    m.push_stereo(&l, &r);
    assert!(
        m.peak_linear() >= discrete,
        "true peak {} < discrete {}",
        m.peak_linear(),
        discrete
    );
}

#[test]
fn silent_input_floors_out() {
    let l = vec![0.0_f32; 2048];
    let r = vec![0.0_f32; 2048];
    let mut m = TruePeakMeter::new();
    m.push_stereo(&l, &r);
    assert!(m.peak_dbtp() < -100.0);
}
