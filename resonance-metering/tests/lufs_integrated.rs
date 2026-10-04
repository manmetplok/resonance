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
    assert!(!acc.cap_reached());
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
    assert!(acc.cap_reached());
    for _ in 0..10 {
        acc.push_block(ms);
    }
    assert_eq!(acc.len(), cap);
    assert_eq!(acc.dropped_blocks(), 11);
    assert!(acc.cap_reached());
    let got = acc.integrated_lufs();
    assert!((got - -20.0).abs() < 1e-6, "got {got}");

    // Reset rearms the accumulator, the drop counter and the cap flag.
    acc.reset();
    assert_eq!(acc.dropped_blocks(), 0);
    assert_eq!(acc.len(), 0);
    assert!(!acc.cap_reached());
}

#[test]
fn block_ms_round_trip() {
    for lufs in [-70.0, -40.0, -23.0, -14.0, 0.0] {
        let ms = lufs_to_ms(lufs);
        let back = block_mean_square_to_lufs(ms);
        assert!((back - lufs).abs() < 1e-10);
    }
}

/// Deterministic pseudo-random block loudness in [-80, -5] LUFS, so the
/// set straddles both the absolute and the relative gate.
fn spread_blocks(n: usize) -> Vec<f64> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let u = (x >> 11) as f64 / (1u64 << 53) as f64;
            lufs_to_ms(-80.0 + 75.0 * u)
        })
        .collect()
}

#[test]
fn live_gate_matches_exact_gate() {
    // RT-03: the realtime readout is an incremental histogram gate; it
    // must agree with the exact two-pass gate over the same blocks.
    for n in [1usize, 7, 100, 3_000, 30_000] {
        let mut acc = IntegratedAccumulator::new();
        for ms in spread_blocks(n) {
            acc.push_block(ms);
        }
        let exact = acc.integrated_lufs();
        let live = acc.integrated_lufs_live();
        if exact.is_finite() {
            assert!((exact - live).abs() < 0.01, "n={n}: exact {exact} live {live}");
        } else {
            assert!(live.is_infinite(), "n={n}: live {live}");
        }
    }
}

#[test]
fn live_gate_matches_steady_and_silent_material() {
    let mut acc = IntegratedAccumulator::new();
    assert!(acc.integrated_lufs_live().is_infinite());
    for _ in 0..50 {
        acc.push_block(0.0);
    }
    assert!(acc.integrated_lufs_live().is_infinite());
    for _ in 0..100 {
        acc.push_block(lufs_to_ms(-23.0));
    }
    let live = acc.integrated_lufs_live();
    assert!((live - -23.0).abs() < 1e-9, "live {live}");
    acc.reset();
    assert!(acc.integrated_lufs_live().is_infinite());
}

#[test]
fn live_gate_never_runs_the_full_history_gate() {
    use resonance_metering::lufs::gating::full_gate_calls_on_this_thread;
    let mut acc = IntegratedAccumulator::new();
    let before = full_gate_calls_on_this_thread();
    for ms in spread_blocks(5_000) {
        acc.push_block(ms);
        let _ = acc.integrated_lufs_live();
    }
    assert_eq!(full_gate_calls_on_this_thread(), before);
    let _ = acc.integrated_lufs();
    assert_eq!(full_gate_calls_on_this_thread(), before + 1);
}

#[test]
fn live_gate_keeps_reading_past_the_session_cap() {
    // The exact list stops at 60 minutes; the histogram has no cap, so a
    // level change after the cap still moves the live reading.
    let mut acc = IntegratedAccumulator::new();
    while acc.dropped_blocks() == 0 {
        acc.push_block(lufs_to_ms(-30.0));
    }
    let cap = acc.len();
    for _ in 0..cap {
        acc.push_block(lufs_to_ms(-20.0));
    }
    let live = acc.integrated_lufs_live();
    assert!(live > -25.0, "live {live}");
}
