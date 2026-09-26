//! The silent playhead advance (lock-contended blocks, and every block
//! while A/B monitors the reference) must wrap at the loop seam the way
//! the rendering path does: `loop_in + overshoot`, not a bare snap to
//! `loop_in` that loses up to a buffer of timeline per pass (code review
//! MIX-11).

use std::sync::atomic::Ordering;

use resonance_audio::test_support::MixAudioHarness;
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;

fn harness() -> MixAudioHarness {
    let h = MixAudioHarness::new(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    let s = h.shared();
    s.loop_in.store(1_000, Ordering::Relaxed);
    s.loop_out.store(2_000, Ordering::Relaxed);
    s.loop_enabled.store(true, Ordering::Relaxed);
    s.playing.store(true, Ordering::Relaxed);
    h
}

#[test]
fn lock_contended_block_at_the_seam_carries_the_overshoot() {
    let mut h = harness();
    // 1 950 + 128 = 2 078: 78 frames past loop_out.
    h.shared().playhead.store(1_950, Ordering::Release);
    h.render_lock_contended();
    assert_eq!(h.shared().playhead.load(Ordering::Acquire), 1_078);
}

#[test]
fn reference_block_at_the_seam_carries_the_overshoot() {
    let mut h = harness();
    h.enable_reference(vec![0.1; SR as usize * 2]);
    h.shared().playhead.store(1_900, Ordering::Release);
    h.render();
    assert_eq!(h.shared().playhead.load(Ordering::Acquire), 1_028);
}

#[test]
fn landing_exactly_on_loop_out_wraps_to_loop_in() {
    let mut h = harness();
    h.shared().playhead.store(2_000 - BLOCK as u64, Ordering::Release);
    h.render_lock_contended();
    assert_eq!(h.shared().playhead.load(Ordering::Acquire), 1_000);
}

#[test]
fn a_loop_shorter_than_a_buffer_stays_inside_the_loop() {
    let mut h = harness();
    h.shared().loop_out.store(1_050, Ordering::Relaxed);
    h.shared().playhead.store(1_040, Ordering::Release);
    h.render_lock_contended();
    let p = h.shared().playhead.load(Ordering::Acquire);
    assert!((1_000..1_050).contains(&p), "playhead {p} must stay in the loop");
}
