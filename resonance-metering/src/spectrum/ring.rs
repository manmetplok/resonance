//! Lock-free single-producer single-consumer sample ring.
//!
//! The audio thread pushes mono f32 samples at block rate; the spectrum
//! worker thread pops them in chunks as it runs FFTs. Power-of-two sized
//! so the index wrap is a cheap bitmask.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Fixed-size lock-free ring buffer for f32 samples.
///
/// **Threading:** exactly one producer thread may call [`SpscRing::push`]
/// and exactly one consumer thread may call [`SpscRing::pop_into`] /
/// [`SpscRing::available`]. Any other concurrent access is unsound.
pub struct SpscRing {
    /// Per-element cells rather than `UnsafeCell<Box<[f32]>>`: deriving the
    /// base pointer from a `&[UnsafeCell<f32>]` never materialises a
    /// `&[f32]` / `&mut [f32]` over the shared bytes (a shared reference to
    /// `UnsafeCell` asserts nothing about its contents), so producer and
    /// consumer can hold pointers into the same allocation concurrently
    /// without an aliasing violation. See [`SpscRing::data_ptr`].
    buffer: Box<[UnsafeCell<f32>]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
    /// Deferred-clear flag. Any thread may set it via
    /// [`SpscRing::request_clear`]; only the consumer services it via
    /// [`SpscRing::take_clear_request`]. This exists so the producer can
    /// ask for a clear without writing `head` itself — a producer-side
    /// `clear()` would race the consumer's `pop_into` (both writing
    /// `head`), letting the producer reuse cells the consumer is mid-read.
    clear_requested: AtomicBool,
}

// Safety: SpscRing is designed for cross-thread SPSC use (`UnsafeCell` is
// !Sync). Access to the cells is gated by the SPSC discipline in
// push / push_slice / pop_into: the head/tail indices keep the two threads
// on disjoint elements, so no element is read and written concurrently.
unsafe impl Send for SpscRing {}
unsafe impl Sync for SpscRing {}

impl SpscRing {
    /// `capacity` must be a power of two.
    pub fn new(capacity: usize) -> Self {
        assert!(
            capacity.is_power_of_two() && capacity >= 2,
            "SpscRing capacity must be a power of two >= 2"
        );
        let buffer = std::iter::repeat_with(|| UnsafeCell::new(0.0_f32))
            .take(capacity)
            .collect();
        Self {
            buffer,
            mask: capacity - 1,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            clear_requested: AtomicBool::new(false),
        }
    }

    /// Base pointer to the sample storage.
    ///
    /// Derived through `&[UnsafeCell<f32>]` without ever creating a
    /// `&[f32]` or `&mut [f32]`: a shared reference to `UnsafeCell`
    /// carries write permission for the contents and makes no
    /// no-other-writers claim, so both threads may derive and use this
    /// pointer concurrently (on disjoint indices). `UnsafeCell<f32>` is
    /// `repr(transparent)`, so element `i` of the cell slice is the
    /// `f32` at `.add(i)` of the cast pointer.
    #[inline]
    fn data_ptr(&self) -> *mut f32 {
        self.buffer.as_ptr() as *mut f32
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
        // Safety: producer is the only thread writing to the buffer, and
        // the head/tail discipline keeps the consumer off this slot until
        // the Release store below. `data_ptr` yields the base pointer
        // without materialising a `&mut [f32]` over the shared bytes —
        // the consumer may be reading a *different* index in the same
        // allocation right now, and a whole-buffer reference here (even a
        // transient one) would be an aliasing violation that Miri flags.
        unsafe {
            self.data_ptr().add(tail & self.mask).write(sample);
        }
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        true
    }

    /// Push a slice. If the ring fills up the excess samples are dropped;
    /// returns the number of samples actually written.
    ///
    /// Unlike a loop over [`SpscRing::push`], this reserves space with a
    /// single Acquire load of `head`, bulk-copies (at most two `memcpy`
    /// segments around the wrap point), and commits with a single Release
    /// store of `tail` — so the consumer either sees none or all of the
    /// pushed samples, and the producer pays two atomics per call instead
    /// of two per sample.
    #[inline]
    pub fn push_slice(&self, samples: &[f32]) -> usize {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        let free = self.capacity() - tail.wrapping_sub(head);
        let n = samples.len().min(free);
        if n == 0 {
            return 0;
        }
        // Safety: producer is the only thread writing to the buffer, and
        // the `free` computation above guarantees the `n` slots starting
        // at `tail` are not visible to the consumer until the Release
        // store below. Base pointer via `data_ptr` — never a whole-buffer
        // reference — for the same aliasing reason documented in `push`.
        unsafe {
            let ptr = self.data_ptr();
            let start = tail & self.mask;
            let first = n.min(self.capacity() - start);
            std::ptr::copy_nonoverlapping(samples.as_ptr(), ptr.add(start), first);
            if n > first {
                std::ptr::copy_nonoverlapping(samples.as_ptr().add(first), ptr, n - first);
            }
        }
        self.tail.store(tail.wrapping_add(n), Ordering::Release);
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
        // Safety: consumer is the only thread reading from the buffer,
        // and the Acquire load of `tail` above publishes the producer's
        // writes to the `n` slots being read. Base pointer via `data_ptr`
        // — see the SAFETY note in `push`: the producer may concurrently
        // write a disjoint index in the same allocation, so no
        // whole-buffer `&[f32]` may exist here even transiently.
        unsafe {
            let ptr = self.data_ptr();
            for (i, slot) in dst.iter_mut().enumerate().take(n) {
                *slot = ptr.add(head.wrapping_add(i) & self.mask).read();
            }
        }
        self.head.store(head.wrapping_add(n), Ordering::Release);
        n
    }

    /// Drop all unread samples. Only the consumer may call this: it writes
    /// `head`, which the SPSC discipline reserves for the consumer. Any
    /// other thread that wants the ring emptied must use
    /// [`SpscRing::request_clear`] instead.
    pub fn clear(&self) {
        let tail = self.tail.load(Ordering::Acquire);
        self.head.store(tail, Ordering::Release);
    }

    /// Ask the consumer to drop all unread samples at its next service
    /// point. Safe to call from any thread (including the producer):
    /// it only sets an atomic flag and never touches `head`/`tail`.
    ///
    /// The clear is deferred until the consumer calls
    /// [`SpscRing::take_clear_request`], so samples pushed between the
    /// request and the service point are dropped along with the pending
    /// ones — acceptable for reset-style semantics.
    pub fn request_clear(&self) {
        self.clear_requested.store(true, Ordering::Release);
    }

    /// Consumer-only: service a pending [`SpscRing::request_clear`], if
    /// any. Returns `true` if a clear was performed, so the consumer can
    /// also reset whatever downstream state it accumulated from samples
    /// that are now discarded. Call this *between* `pop_into` calls —
    /// on the consumer thread it can never interleave a partial pop.
    pub fn take_clear_request(&self) -> bool {
        if self.clear_requested.swap(false, Ordering::Acquire) {
            self.clear();
            true
        } else {
            false
        }
    }
}

