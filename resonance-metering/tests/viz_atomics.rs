//! The shared lock-free viz primitives: single-cell integrity under a
//! concurrent writer (no torn f32), ring-cursor monotonicity, and the
//! chronological round-trip every plugin's history trace relies on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use resonance_metering::viz::{AtomicF32, AtomicF32Array, AtomicF32Pair, AtomicHistoryRing};

const RING_LEN: usize = 256;

/// Two bit patterns whose halves differ everywhere, so any torn mix of
/// them produces a word that is neither.
const PATTERN_A: u32 = 0xAAAA_AAAA;
const PATTERN_B: u32 = 0x5555_5555;

/// Writer hammers a cell and a ring with two distinguishable bit
/// patterns; reader asserts every observed value is exactly one of them
/// (no torn f32) and that the ring cursor never goes backwards.
#[test]
fn cell_and_ring_are_tear_free_under_stress() {
    let cell = Arc::new(AtomicF32::new(f32::from_bits(PATTERN_A)));
    let ring = Arc::new(AtomicHistoryRing::<RING_LEN>::new(f32::from_bits(
        PATTERN_A,
    )));
    let done = Arc::new(AtomicBool::new(false));

    let writer = {
        let cell = cell.clone();
        let ring = ring.clone();
        let done = done.clone();
        thread::spawn(move || {
            let mut flip = false;
            while !done.load(Ordering::Acquire) {
                let bits = if flip { PATTERN_A } else { PATTERN_B };
                flip = !flip;
                let v = f32::from_bits(bits);
                cell.store(v, Ordering::Relaxed);
                ring.push(v);
            }
        })
    };

    let deadline = Instant::now() + Duration::from_millis(300);
    let mut last_pushed = 0usize;
    while Instant::now() < deadline {
        let bits = cell.load(Ordering::Relaxed).to_bits();
        assert!(
            bits == PATTERN_A || bits == PATTERN_B,
            "torn f32 cell: {bits:#010x}"
        );
        for v in ring.iter_chrono() {
            let bits = v.to_bits();
            assert!(
                bits == PATTERN_A || bits == PATTERN_B,
                "torn f32 ring sample: {bits:#010x}"
            );
        }
        let pushed = ring.total_pushed();
        assert!(
            pushed >= last_pushed,
            "cursor went backwards: {pushed} < {last_pushed}"
        );
        last_pushed = pushed;
    }

    done.store(true, Ordering::Release);
    writer.join().unwrap();
    assert!(last_pushed > 0, "writer never pushed");
}

#[test]
fn ring_round_trips_in_chronological_order() {
    let ring = AtomicHistoryRing::<RING_LEN>::new(0.0);
    // A full lap plus a partial second one so the snapshot exercises the
    // wraparound path.
    let total = RING_LEN + RING_LEN / 2;
    for i in 0..total {
        ring.push(i as f32);
    }
    assert_eq!(ring.total_pushed(), total);
    let got: Vec<f32> = ring.iter_chrono().collect();
    assert_eq!(got.len(), RING_LEN);
    let oldest = (total - RING_LEN) as f32;
    for (i, &v) in got.iter().enumerate() {
        assert_eq!(v, oldest + i as f32, "index {i}");
    }
}

#[test]
fn ring_partial_fill_keeps_initial_prefix() {
    let ring = AtomicHistoryRing::<RING_LEN>::new(f32::NEG_INFINITY);
    ring.push(3.0);
    ring.push(6.0);
    let got: Vec<f32> = ring.iter_chrono().collect();
    assert!(got[..RING_LEN - 2].iter().all(|&v| v == f32::NEG_INFINITY));
    assert_eq!(&got[RING_LEN - 2..], &[3.0, 6.0]);
}

#[test]
fn pair_and_array_round_trip() {
    let pair = AtomicF32Pair::new(f32::NEG_INFINITY);
    assert_eq!(pair.load(), (f32::NEG_INFINITY, f32::NEG_INFINITY));
    pair.store(-6.0, -12.0);
    assert_eq!(pair.load(), (-6.0, -12.0));

    let arr = AtomicF32Array::<4>::new(0.0);
    arr.store(&[1.0, 2.0, 3.0, 4.0]);
    assert_eq!(arr.load(), [1.0, 2.0, 3.0, 4.0]);
    arr.store_at(2, 9.0);
    assert_eq!(arr.load_at(2), 9.0);
    assert_eq!(arr.load(), [1.0, 2.0, 9.0, 4.0]);
}
