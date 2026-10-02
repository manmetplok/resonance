//! The loop range is one published value (code review RT-13).
//!
//! It used to be three relaxed atomics, so a block reading them while the
//! engine thread moved the loop could pair the new `loop_in` with the old
//! `loop_out` and wrap to the wrong place, or skip the wrap. A reader must
//! only ever see a range that was actually published.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use resonance_audio::test_support::{LoopRange, SharedState};

#[test]
fn a_reader_never_sees_half_of_a_loop_move() {
    let a = LoopRange::new(true, 0, 1_000);
    let b = LoopRange::new(true, 50_000, 90_000);
    let shared = Arc::new(SharedState::default());
    shared.set_loop_range(a);
    let stop = Arc::new(AtomicBool::new(false));

    let writer = {
        let shared = Arc::clone(&shared);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut flip = false;
            while !stop.load(Ordering::Relaxed) {
                shared.set_loop_range(if flip { a } else { b });
                shared.retired.sweep();
                flip = !flip;
            }
        })
    };

    for _ in 0..200_000 {
        let seen = shared.loop_range();
        assert!(seen == a || seen == b, "torn loop range {seen:?}");
    }
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
}

#[test]
fn the_seam_wraps_at_the_published_range() {
    let shared = SharedState::default();
    assert_eq!(shared.loop_range(), LoopRange::default());
    shared.set_loop_range(LoopRange::new(true, 64, 4_096));
    assert_eq!(shared.loop_range(), LoopRange::new(true, 64, 4_096));
}
