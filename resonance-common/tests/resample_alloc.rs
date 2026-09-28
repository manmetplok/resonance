//! The streaming resampler runs inside the cpal input callback (monitor
//! path), so after construction `process`/`flush` must not allocate —
//! provided the caller's output Vec already has room (LIB-01).
//!
//! Counted only on the thread under test (armed thread-locally), so an
//! allocation elsewhere in the process — libtest's main thread, say —
//! cannot land in the measured window.

use resonance_common::StreamingLinearResampler;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

fn count() {
    if ARMED.try_with(Cell::get).unwrap_or(false) {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count();
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        count();
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn process_and_flush_do_not_allocate() {
    // 44.1->48 uses exact polyphase rows; 44.1->47.999 the interpolated
    // table (too many phases for an exact one). Both paths are checked.
    for (from, to) in [(44_100, 48_000), (44_100, 47_999), (96_000, 48_000)] {
        let input: Vec<f32> = (0..2_048).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut out = Vec::with_capacity(8_192);
        let mut r = StreamingLinearResampler::new(from, to);
        let before = ALLOCS.load(Ordering::SeqCst);
        ARMED.with(|a| a.set(true));
        for c in input.chunks(128) {
            r.process(c, &mut out);
        }
        r.flush(&mut out);
        ARMED.with(|a| a.set(false));
        let after = ALLOCS.load(Ordering::SeqCst);
        assert!(!out.is_empty());
        assert_eq!(after - before, 0, "{from}->{to} allocated");
    }
}
