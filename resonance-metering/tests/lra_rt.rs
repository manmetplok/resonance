//! RT-safety regression: `LraMeter::push_short_term_mean_square` and
//! `lra_lu` run on the audio thread (per block, via `ABMeterTap`), so
//! neither may allocate. The old implementation built two `Vec`s and ran
//! a stable sort per `lra_lu` call — this test pins the fix by counting
//! heap traffic through a wrapping global allocator while the hot path
//! runs on a populated meter.
//!
//! Kept in its own test binary so the counter only ever observes this
//! test; the armed flag is thread-local so harness threads can't trip it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use resonance_metering::LraMeter;

struct CountingAlloc;

static ARMED_HITS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

fn armed() -> bool {
    ARMED.try_with(Cell::get).unwrap_or(false)
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if armed() {
            ARMED_HITS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if armed() {
            ARMED_HITS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAlloc = CountingAlloc;

#[test]
fn push_and_lra_readout_do_not_touch_the_heap() {
    // Populate the meter so the readout walks a
    // large distribution, spread across the
    // whole loudness range (worst case for the old sort path).
    let mut meter = LraMeter::new();
    for i in 0..3600_u32 {
        let lufs = -60.0 + (i % 50) as f64;
        meter.push_short_term_mean_square(10.0_f64.powf((lufs + 0.691) / 10.0));
    }
    let warmup = meter.lra_lu();
    assert!(warmup.is_finite());

    ARMED.with(|a| a.set(true));
    let mut acc = 0.0_f32;
    for i in 0..1000_u32 {
        // Interleave pushes with
        // per-block readouts, mimicking the audio callback's cadence.
        meter.push_short_term_mean_square(10.0_f64.powf((-23.0 + 0.691) / 10.0));
        acc += meter.lra_lu();
        if i == 500 {
            meter.reset();
        }
    }
    ARMED.with(|a| a.set(false));

    let hits = ARMED_HITS.load(Ordering::Relaxed);
    assert_eq!(
        hits, 0,
        "LraMeter hot path performed {hits} heap operations (acc = {acc})"
    );
}
