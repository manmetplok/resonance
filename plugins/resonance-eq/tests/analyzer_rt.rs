//! Real-time-safety audit of the EQ process path with the spectrum
//! analyzer engaged: `process()` must not touch the heap — the analyzer
//! tap is a mono downmix pushed into a preallocated lock-free ring, and
//! the FFT (which does allocate, per published snapshot) lives on the
//! background worker threads.
//!
//! The check counts allocator calls with a wrapping global allocator.
//! This file deliberately contains a SINGLE test: the counter is global,
//! so concurrent tests in the same binary would pollute it. For the same
//! reason the measured region keeps the total samples pushed per tap
//! below the worker's hop size (4096), so the workers — whose per-frame
//! `Arc` publish would also be counted — provably never run an FFT
//! while the audio thread is being measured.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingAllocator;

static ALLOC_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

use resonance_eq::ResonanceEq;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

#[test]
fn process_with_analyzer_never_allocates() {
    // Construction and initialization MAY allocate (DSP state, ring
    // buffers, worker threads); audit only the processing loop.
    let mut plugin = ResonanceEq::new();
    // Enable a couple of bands so the full cascade runs, not a bypass.
    plugin.params.bands[0].enabled.set_value(true);
    plugin.params.bands[0].kind.set_value(3); // low cut
    plugin.params.bands[0].freq.set_value(60.0);
    plugin.params.bands[3].enabled.set_value(true);
    plugin.params.bands[3].kind.set_value(0); // bell
    plugin.params.bands[3].gain.set_value(6.0);
    assert!(plugin.initialize(SR, BLOCK as u32));

    // Let the two freshly spawned worker threads reach their run loop:
    // each allocates its drain scratch once on startup, and that must
    // not land inside the measured window.
    std::thread::sleep(std::time::Duration::from_millis(100));

    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let mut ev = EventIterator::empty();

    // Warm up: covers first-block coefficient configuration and the
    // smoother retarget. 4 + 8 blocks = 3072 samples per tap, below the
    // worker's 4096-sample hop, so no worker-side FFT can run.
    for block in 0..4 {
        run_block(&mut plugin, &mut left, &mut right, &mut ev, block);
    }

    let before = ALLOC_CALLS.load(Ordering::Relaxed);
    for block in 4..12 {
        run_block(&mut plugin, &mut left, &mut right, &mut ev, block);
    }
    let after = ALLOC_CALLS.load(Ordering::Relaxed);

    for &x in left.iter().chain(right.iter()) {
        assert!(x.is_finite(), "output must stay finite: {x}");
    }
    assert_eq!(
        after - before,
        0,
        "process() with the analyzer engaged must not touch the allocator \
         (counted {} calls over 8 blocks)",
        after - before
    );
}

fn run_block(
    plugin: &mut ResonanceEq,
    left: &mut [f32],
    right: &mut [f32],
    ev: &mut EventIterator<'_>,
    block: usize,
) {
    for i in 0..BLOCK {
        let n = (block * BLOCK + i) as f32;
        let s = (std::f32::consts::TAU * 440.0 * n / SR).sin() * 0.4;
        left[i] = s;
        right[i] = -s;
    }
    let mut outs = [OutputBuffer { left, right }];
    plugin.process(&mut outs, BLOCK, ev, None);
}
