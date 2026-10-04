//! Incremental BS.1770-4 gating for a realtime readout.
//!
//! [`gated_integrated_lufs`](super::gating::gated_integrated_lufs) runs
//! two passes over every 100 ms gating block of the session, so its cost
//! grows with how long the meter has been running. A meter read from the
//! audio callback can't afford that (code review RT-03: 400 µs per
//! recompute after an hour, every 100 ms, inside a 2.67 ms quantum).
//!
//! [`IncrementalGate`] keeps the same information as a fixed-size
//! loudness histogram, the way the LRA meter does: each absolute-gated
//! block is an O(1) bucket update on push, and a readout walks only the
//! buckets between the relative gate and the loudest populated bucket.
//! Both are independent of session length, and nothing allocates after
//! construction.
//!
//! What stays exact: the absolute gate, and the relative-gate *reference*
//! (a running energetic mean of the absolute-gated blocks). Every bucket
//! wholly above the relative gate also contributes its exact energy and
//! count. Only the one bucket the gate cuts through is apportioned by the
//! fraction of its span above the gate, so the error is bounded by the
//! energy of one [`BIN_LU`]-wide bucket sitting 10 LU under the reference,
//! far below the meter's display resolution.

use super::gating::{block_mean_square_to_lufs, ABSOLUTE_GATE_LUFS, RELATIVE_GATE_LU};

/// Histogram floor: the absolute gate (blocks below it are never stored).
const MIN_LUFS: f64 = ABSOLUTE_GATE_LUFS;
/// Histogram ceiling; hotter blocks clamp into the top bucket (their
/// energy stays exact, only their bucket position saturates).
const MAX_LUFS: f64 = 10.0;
/// Bucket width in LU.
pub const BIN_LU: f64 = 0.02;
/// Number of buckets over [`MIN_LUFS`, `MAX_LUFS`].
const BINS: usize = ((MAX_LUFS - MIN_LUFS) / BIN_LU) as usize;

/// Streaming integrated-loudness gate with a constant-time push and a
/// readout whose cost does not depend on how many blocks were pushed.
pub struct IncrementalGate {
    /// Per-bucket count of absolute-gated blocks.
    counts: Box<[u32]>,
    /// Per-bucket sum of the blocks' exact mean-squares.
    sums: Box<[f64]>,
    /// Running energy and count of every absolute-gated block: the exact
    /// ungated reference the relative gate is set from.
    abs_sum: f64,
    abs_count: u64,
    /// Highest populated bucket, so a readout stops there.
    top_bin: usize,
}

impl IncrementalGate {
    pub fn new() -> Self {
        Self {
            counts: vec![0u32; BINS].into_boxed_slice(),
            sums: vec![0.0f64; BINS].into_boxed_slice(),
            abs_sum: 0.0,
            abs_count: 0,
            top_bin: 0,
        }
    }

    /// Forget every block. Two fixed-size fills; no allocation.
    pub fn reset(&mut self) {
        self.counts.fill(0);
        self.sums.fill(0.0);
        self.abs_sum = 0.0;
        self.abs_count = 0;
        self.top_bin = 0;
    }

    /// Add one gating block's mean-square. O(1), allocation-free.
    #[inline]
    pub fn push_block(&mut self, mean_square: f64) {
        let lufs = block_mean_square_to_lufs(mean_square);
        if !(lufs >= ABSOLUTE_GATE_LUFS) {
            return;
        }
        let bin = bin_index(lufs);
        self.counts[bin] = self.counts[bin].saturating_add(1);
        self.sums[bin] += mean_square;
        self.abs_sum += mean_square;
        self.abs_count += 1;
        self.top_bin = self.top_bin.max(bin);
    }

    /// Absolute-gated blocks pushed since the last reset.
    pub fn gated_block_count(&self) -> u64 {
        self.abs_count
    }

    /// Gated integrated loudness in LUFS, `f64::NEG_INFINITY` when no
    /// block passed the absolute gate. Walks at most the buckets between
    /// the relative gate and the loudest block — bounded by the histogram
    /// size, never by the session length.
    pub fn integrated_lufs(&self) -> f64 {
        if self.abs_count == 0 {
            return f64::NEG_INFINITY;
        }
        let reference = block_mean_square_to_lufs(self.abs_sum / self.abs_count as f64);
        let threshold = (reference + RELATIVE_GATE_LU).max(ABSOLUTE_GATE_LUFS);
        let first = bin_index(threshold);
        let mut sum = 0.0_f64;
        let mut count = 0.0_f64;
        for bin in first..=self.top_bin {
            let n = self.counts[bin];
            if n == 0 {
                continue;
            }
            let lo = MIN_LUFS + bin as f64 * BIN_LU;
            let frac = if lo >= threshold {
                1.0
            } else {
                ((lo + BIN_LU - threshold) / BIN_LU).clamp(0.0, 1.0)
            };
            sum += self.sums[bin] * frac;
            count += n as f64 * frac;
        }
        if count <= 0.0 {
            return f64::NEG_INFINITY;
        }
        block_mean_square_to_lufs(sum / count)
    }
}

impl Default for IncrementalGate {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn bin_index(lufs: f64) -> usize {
    let clamped = lufs.clamp(MIN_LUFS, MAX_LUFS);
    (((clamped - MIN_LUFS) / BIN_LU) as usize).min(BINS - 1)
}
