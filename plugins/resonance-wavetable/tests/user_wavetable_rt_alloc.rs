//! Real-time safety of the user-wavetable swap: installing an imported table,
//! replacing it and clearing it all happen inside `process()`, which must not
//! touch the heap — neither to take the new table nor to free the old one
//! (a 256-frame table is 25 MB; its free belongs on the janitor thread).
//!
//! Allocator calls are counted by a wrapping global allocator, armed only on
//! the thread driving `process()`: the loader and the janitor legitimately
//! allocate and free at the same time, on their own threads. This file holds
//! a single test so nothing else in the binary shares the counter.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicUsize, Ordering};

use resonance_plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin};
use resonance_wavetable::dsp::wavetable::{USER_WAVETABLE_INDEX, WAVETABLE_SIZE};
use resonance_wavetable::ResonanceWavetable;

struct CountingAllocator;

static ALLOC_CALLS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

fn count() {
    if ARMED.try_with(Cell::get).unwrap_or(false) {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count();
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

/// `frames` frames, each a different harmonic mix.
fn frames(frames: usize, seed: usize) -> Vec<f32> {
    (0..frames * WAVETABLE_SIZE)
        .map(|i| {
            let f = i / WAVETABLE_SIZE;
            let t = TAU * (i % WAVETABLE_SIZE) as f32 / WAVETABLE_SIZE as f32;
            t.sin() + 0.5 * ((2 + (f + seed) % 9) as f32 * t).sin()
        })
        .collect()
}

/// One `process()` call with the counter armed; returns its allocator calls.
fn armed_block(
    plugin: &mut ResonanceWavetable,
    events: &[NoteEvent],
    left: &mut [f32],
    right: &mut [f32],
) -> usize {
    let mut iter = EventIterator::new(events);
    let mut outs = [OutputBuffer { left, right }];
    ALLOC_CALLS.store(0, Ordering::Relaxed);
    ARMED.with(|a| a.set(true));
    plugin.process(&mut outs, BLOCK, &mut iter, None);
    ARMED.with(|a| a.set(false));
    ALLOC_CALLS.load(Ordering::Relaxed)
}

#[test]
fn installing_replacing_and_clearing_a_user_table_never_touches_the_heap() {
    let mut plugin = ResonanceWavetable::new();
    plugin.initialize(SR, BLOCK as u32);
    let osc1 = (0..plugin.param_count())
        .map(|i| plugin.param(i))
        .find(|p| p.id() == "osc1_wavetable")
        .unwrap();
    osc1.set_plain(USER_WAVETABLE_INDEX as f64);

    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let note_on = [NoteEvent::NoteOn {
        note: 48,
        velocity: 0.9,
        timing: 0,
    }];
    // A sounding voice, so the swaps land under a note being played.
    assert_eq!(armed_block(&mut plugin, &note_on, &mut left, &mut right), 0);

    let shared = plugin.user_wavetables().clone();
    // More swaps than the janitor channel holds, so the parking path runs
    // too if the janitor falls behind; the big tables make a free here
    // unmistakable.
    for round in 0..8 {
        let n = if round % 2 == 0 { 64 } else { 3 };
        shared
            .restore_frames(0, "", "t", frames(n, round))
            .expect("build");
        let calls = armed_block(&mut plugin, &[], &mut left, &mut right);
        assert_eq!(calls, 0, "round {round}: installing a table allocated/freed");
        assert_eq!(plugin.engine().user_table(0).map(|t| t.num_frames()), Some(n));
        assert!(
            left.iter().chain(right.iter()).all(|s| s.is_finite()),
            "round {round}: non-finite output"
        );
        assert!(
            left.iter().any(|s| s.abs() > 1e-4),
            "round {round}: the user table rendered silence"
        );
    }

    shared.clear(0);
    assert_eq!(armed_block(&mut plugin, &[], &mut left, &mut right), 0);
    assert!(plugin.engine().user_table(0).is_none());
    // The fallback (bundled table 0) keeps sounding.
    assert!(left.iter().any(|s| s.abs() > 1e-4));
}
