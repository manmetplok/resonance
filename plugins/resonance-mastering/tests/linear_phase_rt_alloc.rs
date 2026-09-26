//! No-allocation guard for the linear-phase filters' audio path
//! (FU-M2a): automating EQ bands must not touch the heap on the audio
//! thread — neither the worker hand-off nor the inline design fallback.
//!
//! Own test binary: it installs a counting global allocator. The count
//! is per thread, so the design worker's own (construction-time)
//! allocations do not pollute it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use resonance_mastering::stages::linear_phase_eq::{
    BandConfig, BandType, DesignWorker, LinearPhaseEq, NUM_BANDS,
};

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

// SAFETY: delegates entirely to `System`; the const-initialised
// thread-local counter never allocates itself.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

fn thread_allocs() -> usize {
    ALLOC_COUNT.with(|c| c.get())
}

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;

fn bands(block: usize) -> [BandConfig; NUM_BANDS] {
    let mut b = [BandConfig::off(); NUM_BANDS];
    for (i, band) in b.iter_mut().enumerate().take(3) {
        *band = BandConfig {
            enabled: true,
            band_type: [BandType::Bell, BandType::LowShelf, BandType::HighShelf][i],
            freq_hz: 200.0 * (i + 1) as f32 + 50.0 * ((block as f32) * 0.1).sin(),
            q: 0.9,
            gain_db: 4.0 * ((block as f32) * 0.07 + i as f32).sin(),
        };
    }
    b
}

#[test]
fn automating_linear_phase_filters_never_allocates() {
    let worker = DesignWorker::spawn();
    let mut eqs = [
        LinearPhaseEq::with_worker(SR, Some(&worker)),
        LinearPhaseEq::with_worker(SR, None),
    ];
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];

    let before = thread_allocs();
    // ~5 s of audio: dozens of hops, a redesign pending at nearly every
    // boundary.
    for block in 0..2000 {
        for (i, (sl, sr)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
            let n = (block * BLOCK + i) as f32;
            *sl = 0.3 * (n * 0.031).sin();
            *sr = 0.3 * (n * 0.017).sin();
        }
        for eq in eqs.iter_mut() {
            eq.process_stereo(&mut l, &mut r, &bands(block));
        }
    }
    let allocs = thread_allocs() - before;

    let designs: u64 = eqs
        .iter()
        .map(|e| e.design_counts())
        .map(|(w, i)| w + i)
        .sum();
    assert!(designs > 20, "the sweep barely redesigned ({designs} designs)");
    assert!(l.iter().any(|v| v.abs() > 1e-3), "output is silent");
    assert_eq!(allocs, 0, "{allocs} allocations on the audio thread");
}
