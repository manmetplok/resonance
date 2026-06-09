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
fn pushing_past_cap_drops_without_panicking() {
    // Sessions longer than the 60-minute cap are not bugs: the
    // accumulator must keep accepting (and counting) pushes without a
    // debug assertion firing, and the reading must stay finite.
    let mut acc = IntegratedAccumulator::new();
    let ms = lufs_to_ms(-20.0);
    let cap = {
        // Fill to the cap; len() stops growing exactly there.
        let mut n = 0usize;
        while acc.dropped_blocks() == 0 {
            acc.push_block(ms);
            n += 1;
        }
        n - 1
    };
    assert_eq!(acc.len(), cap);
    for _ in 0..10 {
        acc.push_block(ms);
    }
    assert_eq!(acc.len(), cap);
    assert_eq!(acc.dropped_blocks(), 11);
    let got = acc.integrated_lufs();
    assert!((got - -20.0).abs() < 1e-6, "got {got}");

    // Reset rearms both the accumulator and the drop counter.
    acc.reset();
    assert_eq!(acc.dropped_blocks(), 0);
    assert_eq!(acc.len(), 0);
}

#[test]
fn block_ms_round_trip() {
    for lufs in [-70.0, -40.0, -23.0, -14.0, 0.0] {
        let ms = lufs_to_ms(lufs);
        let back = block_mean_square_to_lufs(ms);
        assert!((back - lufs).abs() < 1e-10);
    }
}
