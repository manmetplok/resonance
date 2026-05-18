use resonance_metering::lufs::gating::LOUDNESS_OFFSET;
use resonance_metering::LraMeter;

fn lufs_to_ms(lufs: f64) -> f64 {
    10.0_f64.powf((lufs - LOUDNESS_OFFSET) / 10.0)
}

#[test]
fn empty_session_reports_zero_lra() {
    let lra = LraMeter::new();
    assert_eq!(lra.lra_lu(), 0.0);
}

#[test]
fn constant_level_yields_near_zero_lra() {
    let mut lra = LraMeter::new();
    for _ in 0..100 {
        lra.push_short_term_mean_square(lufs_to_ms(-20.0));
    }
    assert!(lra.lra_lu().abs() < 0.1);
}

#[test]
fn step_from_quiet_to_loud_has_lra_near_the_step() {
    // Step sequence: 20→30→20 dBFS input levels (which map to different
    // LUFS values). This exercises the percentile calculation.
    let mut lra = LraMeter::new();
    for _ in 0..10 {
        lra.push_short_term_mean_square(lufs_to_ms(-20.0));
    }
    for _ in 0..10 {
        lra.push_short_term_mean_square(lufs_to_ms(-30.0));
    }
    for _ in 0..10 {
        lra.push_short_term_mean_square(lufs_to_ms(-20.0));
    }
    let v = lra.lra_lu();
    // Expected LRA ≈ 10 LU (the step height); allow a generous band.
    assert!(v > 5.0 && v < 15.0, "LRA = {v}");
}
