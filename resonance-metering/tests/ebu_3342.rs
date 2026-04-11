//! EBU Tech 3342 LRA compliance — a stepped-loudness test.

mod common;

use resonance_metering::LraMeter;

fn lufs_to_ms(lufs: f64) -> f64 {
    // BS.1770 loudness offset.
    10.0_f64.powf((lufs + 0.691) / 10.0)
}

#[test]
fn stepped_minus_20_minus_30_minus_20_yields_near_10_lu() {
    // EBU Tech 3342 test case 3: stepped sequence -20 / -30 / -20 over
    // equal durations should produce an LRA of ~10 LU. We emulate the
    // meter's per-block feed by pushing short-term mean-square values
    // directly, which is the API the mastering plugin uses.
    let mut lra = LraMeter::new();
    for _ in 0..20 {
        lra.push_short_term_mean_square(lufs_to_ms(-20.0));
    }
    for _ in 0..20 {
        lra.push_short_term_mean_square(lufs_to_ms(-30.0));
    }
    for _ in 0..20 {
        lra.push_short_term_mean_square(lufs_to_ms(-20.0));
    }
    let v = lra.lra_lu();
    // Allow the EBU ±1 LU tolerance.
    assert!(
        (v - 10.0).abs() < 1.0,
        "LRA = {v} LU (expected 10 ± 1)"
    );
}
