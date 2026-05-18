use resonance_dsp::dynamics::*;

#[test]
fn below_threshold_is_zero_gr() {
    // Detector well below threshold — hard knee, any slope.
    let gr = soft_knee_gain_reduction_db(-30.0, -20.0, 0.0, 0.0, 0.5);
    assert_eq!(gr, 0.0);
}

#[test]
fn above_threshold_applies_slope() {
    // 10 dB over threshold, 4:1 ratio → slope 0.75 → 7.5 dB GR.
    let slope = 1.0 - 1.0 / 4.0;
    let gr = soft_knee_gain_reduction_db(-10.0, -20.0, 0.0, 0.0, slope);
    assert!((gr - 7.5).abs() < 1e-4);
}

#[test]
fn soft_knee_is_continuous_at_edges() {
    let knee = 6.0;
    let half_knee = knee * 0.5;
    let threshold = -20.0;
    let slope = 0.75;
    // Just below lower knee edge = 0 GR.
    let lower = soft_knee_gain_reduction_db(
        threshold - half_knee - 0.01,
        threshold,
        knee,
        half_knee,
        slope,
    );
    assert!(lower.abs() < 1e-2);
    // At upper knee edge the knee formula should match the linear
    // formula with a tight tolerance.
    let at_edge =
        soft_knee_gain_reduction_db(threshold + half_knee, threshold, knee, half_knee, slope);
    let linear = slope * half_knee;
    assert!(
        (at_edge - linear).abs() < 1e-4,
        "knee {at_edge} vs linear {linear}"
    );
}

#[test]
fn attack_is_faster_than_release() {
    // Attack 1 ms, release 100 ms at 48 kHz.
    let b = Ballistics::from_times(48_000.0, 1.0, 100.0);
    assert!(b.attack_coef < b.release_coef);
}

#[test]
fn envelope_converges_to_target() {
    // Step from 0 dB current to 6 dB target; envelope should climb.
    let b = Ballistics::from_times(48_000.0, 1.0, 100.0);
    let mut cur = 0.0_f32;
    for _ in 0..1000 {
        cur = b.step_envelope(cur, 6.0);
    }
    assert!(cur > 5.9, "cur = {cur}");
}
