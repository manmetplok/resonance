//! Round-trip tests for the lock-free gain-reduction history ring.

use resonance_compressor::viz::{CompressorViz, HISTORY_LEN};

#[test]
fn gr_history_round_trips_in_chronological_order() {
    let viz = CompressorViz::new();
    // Fill the whole ring plus a partial second lap so the snapshot
    // exercises the wraparound path.
    let total = HISTORY_LEN + HISTORY_LEN / 2;
    for i in 0..total {
        viz.push_gr(i as f32);
    }
    let got: Vec<f32> = viz.history.iter_chrono().collect();
    assert_eq!(got.len(), HISTORY_LEN);
    // Oldest surviving sample first, newest last.
    let oldest = (total - HISTORY_LEN) as f32;
    for (i, &v) in got.iter().enumerate() {
        assert_eq!(v, oldest + i as f32, "index {i}");
    }
}

#[test]
fn gr_history_partial_fill_keeps_zero_prefix() {
    let viz = CompressorViz::new();
    viz.push_gr(3.0);
    viz.push_gr(6.0);
    let got: Vec<f32> = viz.history.iter_chrono().collect();
    assert_eq!(got.len(), HISTORY_LEN);
    assert!(got[..HISTORY_LEN - 2].iter().all(|&v| v == 0.0));
    assert_eq!(got[HISTORY_LEN - 2..], [3.0, 6.0]);
}
