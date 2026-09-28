//! Audio thread → editor state for the goniometer and the correlation
//! strip.
//!
//! Everything is a lock-free atomic cell. The goniometer ring packs each
//! output frame's `(L, R)` into one `AtomicU64`, so a point can never pair
//! one frame's left with another's right; the correlation history is the
//! shared [`AtomicHistoryRing`]. Writers are wait-free and allocation-free.

use std::sync::atomic::{AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use resonance_metering::{AtomicF32, AtomicHistoryRing};

use crate::dsp::WidenMode;

/// Goniometer points kept (one per 4 output frames: ≈ 43 ms at 48 kHz).
pub const GONIO_POINTS: usize = 512;
/// Correlation strip length (one value per 1024 frames: ≈ 5.5 s).
pub const CORRELATION_LEN: usize = 256;

pub struct StereoViz {
    points: [AtomicU64; GONIO_POINTS],
    points_pushed: AtomicUsize,
    /// Correlation over the last ~100 ms, newest block.
    correlation: AtomicF32,
    /// Rolling correlation history for the strip.
    pub history: AtomicHistoryRing<CORRELATION_LEN>,
    /// The widening mode the audio thread last ran.
    mode: AtomicI32,
}

fn pack(l: f32, r: f32) -> u64 {
    (l.to_bits() as u64) << 32 | r.to_bits() as u64
}

fn unpack(v: u64) -> (f32, f32) {
    (f32::from_bits((v >> 32) as u32), f32::from_bits(v as u32))
}

impl StereoViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            points: std::array::from_fn(|_| AtomicU64::new(0)),
            points_pushed: AtomicUsize::new(0),
            correlation: AtomicF32::new(0.0),
            history: AtomicHistoryRing::new(0.0),
            mode: AtomicI32::new(WidenMode::Off.index()),
        })
    }

    /// Push one output frame to the goniometer ring. Wait-free.
    #[inline]
    pub fn push_point(&self, l: f32, r: f32) {
        let n = self.points_pushed.load(Ordering::Relaxed);
        self.points[n % GONIO_POINTS].store(pack(l, r), Ordering::Relaxed);
        self.points_pushed.store(n + 1, Ordering::Release);
    }

    /// Push one correlation value to the strip. Wait-free.
    pub fn push_correlation(&self, r: f32) {
        self.history.push(r);
    }

    /// Publish the block's correlation and the mode that produced it.
    pub fn store_block(&self, correlation: f32, mode: WidenMode) {
        self.correlation.store(correlation, Ordering::Relaxed);
        self.mode.store(mode.index(), Ordering::Relaxed);
    }

    /// Clear the traces (plugin reset).
    pub fn clear(&self) {
        for p in &self.points {
            p.store(0, Ordering::Relaxed);
        }
        self.correlation.store(0.0, Ordering::Relaxed);
    }

    /// The goniometer points, oldest first, as `(L, R)`.
    pub fn points(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        let start = self.points_pushed.load(Ordering::Acquire) % GONIO_POINTS;
        (0..GONIO_POINTS).map(move |i| unpack(self.points[(start + i) % GONIO_POINTS].load(Ordering::Relaxed)))
    }

    /// Total goniometer points pushed since construction.
    pub fn points_pushed(&self) -> usize {
        self.points_pushed.load(Ordering::Acquire)
    }

    pub fn correlation(&self) -> f32 {
        self.correlation.load(Ordering::Relaxed)
    }

    pub fn mode(&self) -> WidenMode {
        WidenMode::from_index(self.mode.load(Ordering::Relaxed))
    }

    /// Whether the last processed block ran a mode that puts static
    /// combs into the mono fold ([`WidenMode::is_mono_risk`]).
    pub fn mono_risk(&self) -> bool {
        self.mode().is_mono_risk()
    }
}
