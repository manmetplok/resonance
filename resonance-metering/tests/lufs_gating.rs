use resonance_metering::lufs::gating::{gated_integrated_lufs, LOUDNESS_OFFSET};

/// Convert a target LUFS value to its underlying mean-square energy.
fn lufs_to_ms(lufs: f64) -> f64 {
    10.0_f64.powf((lufs - LOUDNESS_OFFSET) / 10.0)
}

#[test]
fn single_level_signal_reports_that_level() {
    let blocks = vec![lufs_to_ms(-23.0); 200];
    let got = gated_integrated_lufs(&blocks);
    assert!((got - -23.0).abs() < 1e-6, "got {got}");
}

#[test]
fn absolute_gate_drops_sub_70_lufs_blocks() {
    // 100 blocks at -23 LUFS, 100 at -80 LUFS (well below -70).
    let mut blocks = vec![lufs_to_ms(-23.0); 100];
    blocks.extend(vec![lufs_to_ms(-80.0); 100]);
    let got = gated_integrated_lufs(&blocks);
    assert!((got - -23.0).abs() < 1e-6, "got {got}");
}

#[test]
fn relative_gate_drops_quiet_blocks_below_reference_minus_10() {
    // Half at -23, half at -40. The -40 blocks are more than 10 LU below
    // the ungated reference and should be dropped by the relative gate.
    let mut blocks = vec![lufs_to_ms(-23.0); 100];
    blocks.extend(vec![lufs_to_ms(-40.0); 100]);
    let got = gated_integrated_lufs(&blocks);
    assert!((got - -23.0).abs() < 0.1, "got {got}");
}

#[test]
fn silent_input_returns_neg_infinity() {
    let blocks = vec![0.0; 100];
    let got = gated_integrated_lufs(&blocks);
    assert!(got.is_infinite() && got < 0.0);
}

#[test]
fn empty_input_returns_neg_infinity() {
    let got = gated_integrated_lufs(&[]);
    assert!(got.is_infinite() && got < 0.0);
}
