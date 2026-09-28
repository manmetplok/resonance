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

#[test]
fn ballistics_degenerate_sample_rate_stays_finite() {
    // Zero/negative/NaN sample rates must not produce NaN or infinite
    // coefficients; the envelope must remain usable.
    for sr in [0.0_f32, -48_000.0, f32::NAN] {
        let b = Ballistics::from_times(sr, 10.0, 100.0);
        assert!(
            b.attack_coef.is_finite() && b.release_coef.is_finite(),
            "sr={sr}: coefs {:?}",
            (b.attack_coef, b.release_coef)
        );
        assert!((0.0..1.0).contains(&b.attack_coef), "sr={sr}");
        assert!((0.0..1.0).contains(&b.release_coef), "sr={sr}");
        let env = b.step_envelope(0.0, -6.0);
        assert!(env.is_finite() && env <= 0.0, "sr={sr}: env {env}");
    }
}

/// The ducker the delay and the reverb share: exactly unity while off,
/// `amount × DUCK_MAX_GR_DB` down with the detector held over the
/// threshold, and back to exactly unity once released at amount 0.
#[test]
fn ducker_is_transparent_off_and_settles_at_amount_times_max() {
    let mut d = Ducker::new(48_000.0, 5.0, 50.0);
    for _ in 0..1_000 {
        assert_eq!(d.next_gain(0.9, -0.9, 0.0, -30.0).to_bits(), 1.0f32.to_bits());
    }
    let mut g = 1.0;
    for _ in 0..48_000 {
        g = d.next_gain(0.5, 0.1, 0.5, -30.0);
    }
    let got_db = 20.0 * g.log10();
    assert!((got_db + 0.5 * DUCK_MAX_GR_DB).abs() < 0.01, "settled at {got_db} dB");
    assert!((d.gain_reduction_db() - 0.5 * DUCK_MAX_GR_DB).abs() < 0.01);
    // Release at amount 0: recovers, then snaps to exactly 1.0.
    for _ in 0..48_000 {
        g = d.next_gain(0.5, 0.1, 0.0, -30.0);
    }
    assert_eq!(g.to_bits(), 1.0f32.to_bits());
    // A non-finite detector does not poison the envelope.
    let g = d.next_gain(f32::NAN, f32::INFINITY, 1.0, -30.0);
    assert!(g.is_finite());
}
