//! Lock-free single-producer single-consumer sample ring.
//!
//! The audio thread pushes mono f32 samples at block rate; the spectrum
//! worker thread pops them in chunks as it runs FFTs. Power-of-two sized
//! so the index wrap is a cheap bitmask.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Fixed-size lock-free ring buffer for f32 samples.
///
/// **Threading:** exactly one producer thread may call [`SpscRing::push`]
/// and exactly one consumer thread may call [`SpscRing::pop_into`] /
/// [`SpscRing::available`]. Any other concurrent access is unsound.
pub struct SpscRing {
    buffer: UnsafeCell<Box<[f32]>>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
}

// Safety: SpscRing is designed for cross-thread SPSC use. Access to the
// UnsafeCell is gated by the SPSC discipline in push / pop_into.
unsafe impl Send for SpscRing {}
unsafe impl Sync for SpscRing {}

impl SpscRing {
    /// `capacity` must be a power of two.
    pub fn new(capacity: usize) -> Self {
        assert!(
            capacity.is_power_of_two() && capacity >= 2,
            "SpscRing capacity must be a power of two >= 2"
        );
        let buffer = vec![0.0_f32; capacity].into_boxed_slice();
        Self {
            buffer: UnsafeCell::new(buffer),
            mask: capacity - 1,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Total capacity in samples.
    pub fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// Number of samples available to the consumer right now.
    pub fn available(&self) -> usize {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Relaxed);
        tail.wrapping_sub(head)
    }

    /// Push a single sample. If the ring is full the new sample is
    /// silently dropped — this keeps SPSC thread-safety strict (only the
    /// consumer ever writes `head`). The ring is sized at construction so
    /// the worker thread's latency cannot realistically fill it.
    #[inline]
    pub fn push(&self, sample: f32) -> bool {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        let used = tail.wrapping_sub(head);
        if used > self.mask {
            return false;
        }
        // Safety: producer is the only thread writing to the buffer.
        // We use raw pointer arithmetic instead of constructing a
        // `&mut [f32]`, because the consumer may simultaneously hold a
        // `&[f32]` to a *different* index in the same allocation. Even
        // though the indices don't overlap, materialising both `&` and
        // `&mut` to the same allocation through `UnsafeCell::get()` is
        // a Stacked/Tree Borrows violation that Miri flags.
        unsafe {
            let ptr = (*self.buffer.get()).as_mut_ptr();
            ptr.add(tail & self.mask).write(sample);
        }
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        true
    }

    /// Push a slice. Stops early if the ring fills up and returns the
    /// number of samples actually written.
    #[inline]
    pub fn push_slice(&self, samples: &[f32]) -> usize {
        let mut n = 0;
        for &s in samples {
            if !self.push(s) {
                break;
            }
            n += 1;
        }
        n
    }

    /// Copy up to `dst.len()` available samples into `dst` and advance the
    /// consumer index. Returns how many samples were copied.
    pub fn pop_into(&self, dst: &mut [f32]) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let available = tail.wrapping_sub(head);
        let n = dst.len().min(available);
        if n == 0 {
            return 0;
        }
        // Safety: consumer is the only thread reading from the buffer.
        // Raw pointer reads here mirror the producer's raw-pointer write
        // path — see the SAFETY note in `push`. The producer may
        // concurrently write a disjoint index in the same allocation;
        // constructing a `&[f32]` here would alias an exclusive write
        // borrow through Tree Borrows.
        unsafe {
            let ptr = (*self.buffer.get()).as_ptr();
            for (i, slot) in dst.iter_mut().enumerate().take(n) {
                *slot = ptr.add(head.wrapping_add(i) & self.mask).read();
            }
        }
        self.head.store(head.wrapping_add(n), Ordering::Release);
        n
    }

    /// Drop all unread samples. Only the consumer may call this.
    pub fn clear(&self) {
        let tail = self.tail.load(Ordering::Acquire);
        self.head.store(tail, Ordering::Release);
    }
}

