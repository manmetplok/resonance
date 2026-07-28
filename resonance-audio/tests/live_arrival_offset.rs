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

// -- Audio-thread pickup at small quanta (doc #260 finding #16) --------------
//
// The old engine-thread pickup ran ~16 ms after arrival, so at q=128
// (2.67 ms) `block_len - elapsed` always clamped to 0 and every live
// note quantized to the block boundary. Picked up on the audio callback
// one quantum after arrival, elapsed stays under one block and offsets
// spread properly.

const SMALL_BLOCK: usize = 128;

#[test]
fn quantum_128_offsets_are_nonzero_and_spread() {
    let now = Instant::now();
    // Arrived 1 ms ago (48 samples): lands at 128 - 48 = 80.
    let off = live_arrival_sample_offset(now - secs_for_samples(48.0), now, SR, SMALL_BLOCK);
    assert_eq!(off, 80);
    // Two events 0.5 ms apart keep their 24-sample spacing.
    let a = live_arrival_sample_offset(now - secs_for_samples(72.0), now, SR, SMALL_BLOCK);
    let b = live_arrival_sample_offset(now - secs_for_samples(48.0), now, SR, SMALL_BLOCK);
    assert_eq!(b - a, 24);
    assert!(a > 0, "sub-block arrivals must not clamp to the boundary");
}

#[test]
fn engine_cadence_arrivals_would_still_clamp_at_small_quanta() {
    // Documents WHY pickup moved to the audio thread: a 16 ms-old
    // arrival (the old engine cadence) exceeds the whole 128-frame
    // block, so its offset clamps to 0 — timing quantized away.
    let now = Instant::now();
    let arrival = now - std::time::Duration::from_millis(16);
    assert_eq!(live_arrival_sample_offset(arrival, now, SR, SMALL_BLOCK), 0);
}

// -- Instrument resolution for the audio-thread pickup -----------------------

use indexmap::IndexMap;
use resonance_audio::__test_support::live_instrument_for;
use resonance_audio::types::{Track, TrackId, TrackType};

#[test]
fn live_instrument_resolution_mirrors_the_engine_path() {
    let mut tracks: IndexMap<TrackId, Track> = IndexMap::new();
    // Instrument track with an instrument in slot 0.
    let inst = Track::with_type(1, "inst".into(), TrackType::Instrument);
    inst.push_plugin(42);
    inst.push_plugin(43); // FX after the instrument — never the target
    tracks.insert(1, inst);
    // Instrument track with an empty chain.
    tracks.insert(2, Track::with_type(2, "empty".into(), TrackType::Instrument));
    // Audio track with plugins: never accepts live MIDI.
    let audio = Track::new(3, "audio".into());
    audio.push_plugin(7);
    tracks.insert(3, audio);

    assert_eq!(live_instrument_for(&tracks, 1), Some(42));
    assert_eq!(live_instrument_for(&tracks, 2), None);
    assert_eq!(live_instrument_for(&tracks, 3), None);
    assert_eq!(live_instrument_for(&tracks, 99), None);
}
