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
        if used >= self.mask + 1 {
            return false;
        }
        // Safety: producer is the only thread writing to the buffer.
        unsafe {
            let buf = &mut *self.buffer.get();
            buf[tail & self.mask] = sample;
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
        unsafe {
            let buf = &*self.buffer.get();
            for i in 0..n {
                dst[i] = buf[head.wrapping_add(i) & self.mask];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_pop_round_trip() {
        let ring = SpscRing::new(16);
        for i in 0..10 {
            ring.push(i as f32);
        }
        assert_eq!(ring.available(), 10);
        let mut dst = [0.0_f32; 16];
        let n = ring.pop_into(&mut dst);
        assert_eq!(n, 10);
        for i in 0..10 {
            assert_eq!(dst[i], i as f32);
        }
        assert_eq!(ring.available(), 0);
    }

    #[test]
    fn full_ring_drops_new_pushes() {
        let ring = SpscRing::new(4);
        let mut accepted = 0;
        for i in 0..10 {
            if ring.push(i as f32) {
                accepted += 1;
            }
        }
        assert_eq!(accepted, 4);
        // The first four samples are the ones that made it in.
        let mut dst = [0.0_f32; 4];
        let n = ring.pop_into(&mut dst);
        assert_eq!(n, 4);
        assert_eq!(dst, [0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn wraps_around_zero() {
        let ring = SpscRing::new(8);
        // Fill, drain, fill again — exercises wrap arithmetic.
        for i in 0..6 {
            ring.push(i as f32);
        }
        let mut dst = [0.0_f32; 8];
        let n = ring.pop_into(&mut dst);
        assert_eq!(n, 6);
        for i in 10..14 {
            ring.push(i as f32);
        }
        let n = ring.pop_into(&mut dst);
        assert_eq!(n, 4);
        assert_eq!(&dst[..4], &[10.0, 11.0, 12.0, 13.0]);
    }
}
