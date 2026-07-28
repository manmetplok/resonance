//! Offline-export latency polish (doc #260 findings #8 + #18):
//!
//! * `chunk_span` — the shared bounce chunking helper — must pad tail
//!   chunks up to the CLAP activation minimum (`min_frames = 32`) while
//!   consuming only the frames that actually belong to the render
//!   range, so no offline `process()` call ever violates the
//!   activation contract and output lengths stay exact.
//! * `master_chain_latency` — the pure master-FX latency summation the
//!   export paths pre-roll/trim by — must sum the chain only while the
//!   master FX are engaged (a bypassed chain is skipped entirely by the
//!   chunk render).

use std::collections::HashMap;

use resonance_audio::__test_support::{
    chunk_span, master_chain_latency, BOUNCE_CHUNK, MIN_CLAP_FRAMES,
};

#[test]
fn chunk_span_full_chunks_pass_through() {
    // Plenty remaining: render and consume a full chunk.
    assert_eq!(chunk_span(10 * BOUNCE_CHUNK as u64), (BOUNCE_CHUNK, BOUNCE_CHUNK));
    assert_eq!(chunk_span(BOUNCE_CHUNK as u64), (BOUNCE_CHUNK, BOUNCE_CHUNK));
    // Mid-size tails above the CLAP minimum render exactly what remains.
    assert_eq!(chunk_span(500), (500, 500));
    assert_eq!(chunk_span(MIN_CLAP_FRAMES as u64), (MIN_CLAP_FRAMES, MIN_CLAP_FRAMES));
    assert_eq!(chunk_span(MIN_CLAP_FRAMES as u64 + 1), (MIN_CLAP_FRAMES + 1, MIN_CLAP_FRAMES + 1));
}

#[test]
fn chunk_span_pads_short_tails_to_clap_minimum() {
    // A tail below min_frames renders the CLAP minimum but consumes
    // only the remaining frames — the padding is rendered + discarded.
    for remaining in 1..MIN_CLAP_FRAMES as u64 {
        let (render, emit) = chunk_span(remaining);
        assert_eq!(render, MIN_CLAP_FRAMES, "remaining {remaining}");
        assert_eq!(emit, remaining as usize, "remaining {remaining}");
    }
}

#[test]
fn chunk_span_invariants_hold_across_a_range() {
    // emit <= render <= BOUNCE_CHUNK, render >= MIN, and a loop
    // advancing by `emit` always terminates (emit >= 1 while
    // remaining >= 1).
    for remaining in 1..(3 * BOUNCE_CHUNK as u64) {
        let (render, emit) = chunk_span(remaining);
        assert!(emit >= 1);
        assert!(emit <= render);
        assert!(render <= BOUNCE_CHUNK.max(MIN_CLAP_FRAMES));
        assert!(render >= MIN_CLAP_FRAMES);
        assert!(emit as u64 <= remaining);
    }
}

#[test]
fn master_chain_latency_sums_only_when_engaged() {
    let lat: HashMap<u64, u64> = [(1u64, 128u64), (2, 18_672)].into();
    let resolve = |id| lat.get(&id).copied().unwrap_or(0);

    // Engaged: the export must pre-roll/trim by the full chain sum.
    assert_eq!(master_chain_latency(&[1, 2], false, resolve), 18_800);
    // Bypassed: the chunk render skips the chain — no shift to trim.
    assert_eq!(master_chain_latency(&[1, 2], true, resolve), 0);
    // Empty chain / unknown plugins contribute nothing.
    assert_eq!(master_chain_latency(&[], false, resolve), 0);
    assert_eq!(master_chain_latency(&[99], false, resolve), 0);
}
