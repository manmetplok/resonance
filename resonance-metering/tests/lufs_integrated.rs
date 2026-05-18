use resonance_metering::lufs::gating::{block_mean_square_to_lufs, LOUDNESS_OFFSET};
use resonance_metering::lufs::integrated::IntegratedAccumulator;

fn lufs_to_ms(lufs: f64) -> f64 {
    10.0_f64.powf((lufs - LOUDNESS_OFFSET) / 10.0)
}

#[test]
fn new_accumulator_reports_neg_infinity() {
    let acc = IntegratedAccumulator::new();
    assert!(acc.integrated_lufs().is_infinite());
}

#[test]
fn pushes_and_gates_produce_expected_loudness() {
    let mut acc = IntegratedAccumulator::new();
    for _ in 0..100 {
        acc.push_block(lufs_to_ms(-20.0));
    }
    let got = acc.integrated_lufs();
    assert!((got - -20.0).abs() < 1e-6, "got {got}");
}

#[test]
fn reset_clears_all_state() {
    let mut acc = IntegratedAccumulator::new();
    for _ in 0..10 {
        acc.push_block(1.0);
    }
    acc.reset();
    assert_eq!(acc.len(), 0);
    assert!(acc.integrated_lufs().is_infinite());
}

#[test]
fn block_ms_round_trip() {
    for lufs in [-70.0, -40.0, -23.0, -14.0, 0.0] {
        let ms = lufs_to_ms(lufs);
        let back = block_mean_square_to_lufs(ms);
        assert!((back - lufs).abs() < 1e-10);
    }
}
