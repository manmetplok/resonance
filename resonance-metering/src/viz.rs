//! Lock-free audio-thread → editor visualization primitives.
//!
//! Every plugin editor reads live meter state the audio thread publishes,
//! and every plugin used to re-implement the same protocol by hand: f32
//! values bit-punned into `AtomicU32` cells, fixed arrays of such cells,
//! and a fixed-length history ring with an atomic write cursor. This
//! module is the single home for those shapes so an ordering fix or a
//! protocol change lands once instead of once per plugin.
//!
//! All writers are wait-free and allocation-free — safe to call from the
//! audio thread every block. Readers are wait-free too; a reader may
//! observe values from adjacent blocks (cross-cell tearing), which is the
//! accepted trade-off for meter-rate visualization. A single cell can
//! never tear: each f32 is one aligned `AtomicU32` load/store.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// A single f32 cell, bit-punned into an `AtomicU32`.
///
/// Mirrors the std atomics' API — callers pick the ordering — so a plugin
/// that publishes configuration with Release/Acquire (e.g. an engine
/// block size the reader gates on) and one that publishes meters with
/// Relaxed both map onto the same type without weakening either.
pub struct AtomicF32 {
    bits: AtomicU32,
}

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self {
            bits: AtomicU32::new(v.to_bits()),
        }
    }

    /// Store `v`. Wait-free; audio-thread safe.
    #[inline]
    pub fn store(&self, v: f32, order: Ordering) {
        self.bits.store(v.to_bits(), order);
    }

    /// Load the current value. Never torn.
    #[inline]
    pub fn load(&self, order: Ordering) -> f32 {
        f32::from_bits(self.bits.load(order))
    }
}

impl Default for AtomicF32 {
    fn default() -> Self {
        Self::new(0.0)
    }
}

/// A left/right pair of [`AtomicF32`] cells — the shape of every stereo
/// peak-meter readout (`in_l_db`/`in_r_db`, `out_l_db`/`out_r_db`, …).
///
/// Meters are always published Relaxed: each channel is individually
/// fresh, and a reader straddling a writer sees at most a one-block skew
/// between the channels.
pub struct AtomicF32Pair {
    l: AtomicF32,
    r: AtomicF32,
}

impl AtomicF32Pair {
    /// Build a pair with both channels at `initial` (peak meters want
    /// `-inf` so a fresh plugin renders as silence).
    pub fn new(initial: f32) -> Self {
        Self {
            l: AtomicF32::new(initial),
            r: AtomicF32::new(initial),
        }
    }

    /// Store both channels. Wait-free; audio-thread safe.
    #[inline]
    pub fn store(&self, l: f32, r: f32) {
        self.l.store(l, Ordering::Relaxed);
        self.r.store(r, Ordering::Relaxed);
    }

    /// Load `(left, right)`. Each channel is individually tear-free.
    #[inline]
    pub fn load(&self) -> (f32, f32) {
        (
            self.l.load(Ordering::Relaxed),
            self.r.load(Ordering::Relaxed),
        )
    }
}

/// A fixed array of [`AtomicF32`] cells — echo-tap tables, FDN channel
/// energies, coarse buffer-peak bins, per-band gain reduction.
///
/// Like [`AtomicF32Pair`], the array is meter data and is published
/// Relaxed: each element is individually tear-free, and a reader may see
/// elements from adjacent writer passes.
pub struct AtomicF32Array<const N: usize> {
    cells: [AtomicF32; N],
}

impl<const N: usize> AtomicF32Array<N> {
    /// Build an array with every element at `initial`.
    pub fn new(initial: f32) -> Self {
        Self {
            cells: std::array::from_fn(|_| AtomicF32::new(initial)),
        }
    }

    /// Store the whole array. Wait-free; audio-thread safe.
    pub fn store(&self, values: &[f32; N]) {
        for (cell, &v) in self.cells.iter().zip(values.iter()) {
            cell.store(v, Ordering::Relaxed);
        }
    }

    /// Load the whole array into a fresh stack copy.
    pub fn load(&self) -> [f32; N] {
        std::array::from_fn(|i| self.cells[i].load(Ordering::Relaxed))
    }

    /// Store one element. Panics if `i >= N`, like slice indexing.
    #[inline]
    pub fn store_at(&self, i: usize, v: f32) {
        self.cells[i].store(v, Ordering::Relaxed);
    }

    /// Load one element. Panics if `i >= N`, like slice indexing.
    #[inline]
    pub fn load_at(&self, i: usize) -> f32 {
        self.cells[i].load(Ordering::Relaxed)
    }
}

/// Wait-free SPSC ring of f32 samples for rolling history traces
/// (gain-reduction history, wet-RMS tails, LUFS traces).
///
/// The audio thread is the sole producer via [`push`](Self::push); the
/// editor reads at its own cadence via [`iter_chrono`](Self::iter_chrono).
/// Each sample is a single aligned `AtomicU32` load/store so a value is
/// never torn; a read pass may straddle one producer update, which a
/// meter trace tolerates.
///
/// The cursor counts *total pushes* and only ever grows — the write slot
/// is `total % N` — so a consumer can also observe progress (and tests
/// can assert monotonicity) without decoding wraparound.
pub struct AtomicHistoryRing<const N: usize> {
    samples: [AtomicU32; N],
    /// Total samples pushed since construction. Monotonic.
    pushed: AtomicUsize,
}

impl<const N: usize> AtomicHistoryRing<N> {
    /// Build a fresh ring pre-filled with `initial` (GR traces want 0.0,
    /// LUFS traces want `-inf`, TP traces want their dB floor — so an
    /// empty ring renders as silence).
    pub fn new(initial: f32) -> Self {
        let bits = initial.to_bits();
        Self {
            samples: std::array::from_fn(|_| AtomicU32::new(bits)),
            pushed: AtomicUsize::new(0),
        }
    }

    /// Push one sample. Single-producer, wait-free; no allocation, no
    /// locks — audio-thread safe.
    #[inline]
    pub fn push(&self, v: f32) {
        let pushed = self.pushed.load(Ordering::Relaxed);
        self.samples[pushed % N].store(v.to_bits(), Ordering::Relaxed);
        // Release so a consumer's Acquire on the cursor observes the
        // sample store.
        self.pushed.store(pushed + 1, Ordering::Release);
    }

    /// Iterate the ring in chronological order (oldest sample first).
    /// Wait-free consumer.
    pub fn iter_chrono(&self) -> impl Iterator<Item = f32> + '_ {
        let start = self.pushed.load(Ordering::Acquire) % N;
        (0..N).map(move |i| f32::from_bits(self.samples[(start + i) % N].load(Ordering::Relaxed)))
    }

    /// Total samples pushed since construction. Monotonic — never
    /// decreases, regardless of ring wraparound.
    pub fn total_pushed(&self) -> usize {
        self.pushed.load(Ordering::Acquire)
    }
}
