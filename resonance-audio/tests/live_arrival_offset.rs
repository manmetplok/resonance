//! Tests for the live-input arrival → intra-block sample offset
//! conversion. Live MIDI events used to be delivered with
//! `sample_offset 0`, quantizing note timing to the engine-loop/block
//! cadence; `live_arrival_sample_offset` maps the wall-clock arrival
//! to a best-effort position inside the next audio block so relative
//! timing between notes is preserved.

use std::time::{Duration, Instant};

use resonance_audio::live_arrival_sample_offset;

const SR: u32 = 48_000;
const BLOCK: usize = 1024;

fn secs_for_samples(samples: f64) -> Duration {
    Duration::from_secs_f64(samples / SR as f64)
}

#[test]
fn just_arrived_lands_near_end_of_block() {
    let now = Instant::now();
    let off = live_arrival_sample_offset(now, now, SR, BLOCK);
    assert_eq!(off, (BLOCK - 1) as u32);
}

#[test]
fn full_block_old_arrival_lands_at_offset_zero() {
    let now = Instant::now();
    let arrival = now - secs_for_samples(BLOCK as f64);
    let off = live_arrival_sample_offset(arrival, now, SR, BLOCK);
    assert_eq!(off, 0);
}

#[test]
fn older_than_a_block_clamps_to_zero() {
    let now = Instant::now();
    let arrival = now - secs_for_samples(BLOCK as f64 * 5.0);
    let off = live_arrival_sample_offset(arrival, now, SR, BLOCK);
    assert_eq!(off, 0);
}

#[test]
fn half_block_old_arrival_lands_mid_block() {
    let now = Instant::now();
    let arrival = now - secs_for_samples(BLOCK as f64 / 2.0);
    let off = live_arrival_sample_offset(arrival, now, SR, BLOCK);
    // Float round-trip through Duration may land one sample off.
    let mid = (BLOCK / 2) as u32;
    assert!(off >= mid - 1 && off <= mid + 1, "off = {off}");
}

#[test]
fn arrival_after_now_saturates_to_end_of_block() {
    // Clock skew / reordering: arrival "in the future" must not panic
    // or underflow — it saturates to zero elapsed time.
    let now = Instant::now();
    let arrival = now + Duration::from_millis(5);
    let off = live_arrival_sample_offset(arrival, now, SR, BLOCK);
    assert_eq!(off, (BLOCK - 1) as u32);
}

#[test]
fn earlier_arrival_never_gets_larger_offset() {
    // Monotonicity preserves on/off ordering: an event that arrived
    // earlier must never be scheduled after a later one.
    let now = Instant::now();
    let mut prev = 0u32;
    for samples_ago in (0..=2048).rev().step_by(64) {
        let arrival = now - secs_for_samples(samples_ago as f64);
        let off = live_arrival_sample_offset(arrival, now, SR, BLOCK);
        assert!(off >= prev, "offset decreased: {off} < {prev}");
        prev = off;
    }
}

#[test]
fn offset_always_inside_block() {
    let now = Instant::now();
    for samples_ago in [0.0, 0.5, 100.0, 1023.0, 1024.0, 9999.0] {
        let arrival = now - secs_for_samples(samples_ago);
        let off = live_arrival_sample_offset(arrival, now, SR, BLOCK);
        assert!((off as usize) < BLOCK, "off = {off}");
    }
}

#[test]
fn zero_block_len_returns_zero() {
    let now = Instant::now();
    assert_eq!(live_arrival_sample_offset(now, now, SR, 0), 0);
}
