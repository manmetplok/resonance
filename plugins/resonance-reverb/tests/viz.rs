//! Round-trip tests for the lock-free tail-RMS history ring.

use resonance_reverb::viz::{ReverbViz, TAIL_HISTORY_LEN};

#[test]
fn tail_history_round_trips_in_chronological_order() {
    let viz = ReverbViz::new();
    // Fill the whole ring plus a partial second lap so the snapshot
    // exercises the wraparound path.
    let total = TAIL_HISTORY_LEN + TAIL_HISTORY_LEN / 2;
    for i in 0..total {
        viz.push_tail_rms(i as f32);
    }
    let got: Vec<f32> = viz.tail.iter_chrono().collect();
    assert_eq!(got.len(), TAIL_HISTORY_LEN);
    // Oldest surviving sample first, newest last.
    let oldest = (total - TAIL_HISTORY_LEN) as f32;
    for (i, &v) in got.iter().enumerate() {
        assert_eq!(v, oldest + i as f32, "index {i}");
    }
}

#[test]
fn tail_history_partial_fill_keeps_zero_prefix() {
    let viz = ReverbViz::new();
    viz.push_tail_rms(1.0);
    viz.push_tail_rms(2.0);
    let got: Vec<f32> = viz.tail.iter_chrono().collect();
    assert_eq!(got.len(), TAIL_HISTORY_LEN);
    assert!(got[..TAIL_HISTORY_LEN - 2].iter().all(|&v| v == 0.0));
    assert_eq!(got[TAIL_HISTORY_LEN - 2..], [1.0, 2.0]);
}
