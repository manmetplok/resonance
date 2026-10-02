//! Loudness Range (LRA) per EBU Tech 3342.
//!
//! The LRA is computed from the distribution of 3-second ungated short-
//! term loudness measurements across a session:
//!
//! 1. Accumulate short-term blocks every 1 s (3 s window, 2 s overlap).
//! 2. Absolute-gate at -70 LUFS.
//! 3. Compute the *integrated loudness* of the absolute-gated set —
//!    i.e. `block_mean_square_to_lufs(mean(mean-squares))`, NOT a
//!    percentile of the per-block LUFS values. This is the same
//!    energetic mean used to seed the relative gate of the integrated
//!    loudness calculation, just with a -20 LU offset (EBU 3342) rather
//!    than -10 LU.
//! 4. Relative-gate at `integrated_abs - 20 LU`.
//! 5. Report LRA = p95 - p10 of the remaining set.
//!
//! `lra_lu()` is called from the audio callback once per block (~375 Hz
//! at 48 kHz / q128 via `ABMeterTap::snapshot`), so it must not allocate
//! or sort. Instead of keeping the raw block list, the meter maintains a
//! fixed-size loudness histogram incrementally: each accepted block is an
//! O(1) bucket increment on push, and the gate + percentiles are read
//! back from the histogram in O(buckets) with zero allocation. The
//! relative-gate *reference* stays exact (a running energetic mean of the
//! absolute-gated mean-squares); only the percentile positions are
//! quantised, to well under [`HIST_BIN_LU`] — each bucket also tracks the
//! exact mean loudness of its blocks, so discrete-level material (e.g.
//! the EBU 3342 step sequences) reads back bit-for-bit.

use crate::lufs::gating::{block_mean_square_to_lufs, ABSOLUTE_GATE_LUFS};

/// Relative gate offset used by the LRA calculation (distinct from the
/// -10 LU relative gate used for integrated loudness).
pub const LRA_RELATIVE_GATE_LU: f64 = -20.0;

/// Histogram floor. Blocks below the absolute gate are never stored, so
/// the floor coincides with [`ABSOLUTE_GATE_LUFS`].
const HIST_MIN_LUFS: f64 = ABSOLUTE_GATE_LUFS;
/// Histogram ceiling; hotter blocks clamp into the top bucket. BS.1770
/// loudness of a full-scale stereo signal tops out around +2 LUFS, so
/// +10 leaves headroom for out-of-spec material.
const HIST_MAX_LUFS: f64 = 10.0;
/// Bucket width in LU. Percentile quantisation error is bounded by one
/// bucket per percentile, so worst-case LRA deviation from the exact
/// sort-based computation is ~2x this.
const HIST_BIN_LU: f64 = 0.05;
/// Number of histogram buckets ([-70, +10] LUFS at 0.05 LU).
const HIST_BINS: usize = ((HIST_MAX_LUFS - HIST_MIN_LUFS) / HIST_BIN_LU) as usize;

/// Streaming LRA tracker.
pub struct LraMeter {
    /// Per-bucket count of absolute-gated blocks, bucketed on block LUFS.
    counts: Box<[u32]>,
    /// Per-bucket sum of the exact block LUFS values, so a bucket reads
    /// back as the mean of what actually landed in it rather than its
    /// midpoint.
    lufs_sums: Box<[f64]>,
    /// Running sum of the absolute-gated blocks' mean-squares — the exact
    /// energetic mean that seeds the relative gate.
    abs_sum_ms: f64,
    /// Number of absolute-gated blocks.
    ///
    /// There is no session cap: the histogram is constant memory, so a
    /// block of any session length is a bucket increment. (A cap used to
    /// silently drop every block past 3600 pushes, which at the bounce
    /// measurer's 10 Hz cadence froze the LRA after 6 minutes.)
    abs_count: usize,
}

impl LraMeter {
    pub fn new() -> Self {
        Self {
            counts: vec![0u32; HIST_BINS].into_boxed_slice(),
            lufs_sums: vec![0.0f64; HIST_BINS].into_boxed_slice(),
            abs_sum_ms: 0.0,
            abs_count: 0,
        }
    }

    pub fn reset(&mut self) {
        self.counts.fill(0);
        self.lufs_sums.fill(0.0);
        self.abs_sum_ms = 0.0;
        self.abs_count = 0;
    }

    /// Record a 3-second short-term mean-square. Intended to be called at
    /// ~1 Hz from the LUFS meter's host (see `LufsMeter`). O(1), no
    /// allocation: the absolute gate is applied here and the surviving
    /// block becomes a bucket increment plus two running sums.
    pub fn push_short_term_mean_square(&mut self, mean_square: f64) {
        let lufs = block_mean_square_to_lufs(mean_square);
        if lufs >= ABSOLUTE_GATE_LUFS {
            self.abs_sum_ms += mean_square;
            self.abs_count += 1;
            let bin = bin_index(lufs);
            self.counts[bin] += 1;
            self.lufs_sums[bin] += lufs;
        }
    }

    /// Compute LRA in LU. Returns 0.0 for an empty / silent session so
    /// the UI has a sane default.
    ///
    /// Allocation-free and O([`HIST_BINS`]) — safe to call from the audio
    /// thread every block. Per EBU 3342 the relative gate threshold is
    /// `integrated_loudness(abs_gated) - 20 LU`, where the integrated
    /// loudness is the LUFS of the *mean of mean-squares* — not a
    /// percentile of the per-block LUFS values.
    pub fn lra_lu(&self) -> f32 {
        if self.abs_count == 0 {
            return 0.0;
        }
        let reference_lufs = block_mean_square_to_lufs(self.abs_sum_ms / self.abs_count as f64);
        let threshold = reference_lufs + LRA_RELATIVE_GATE_LU;

        let mut total = 0.0_f64;
        for bin in 0..HIST_BINS {
            total += self.effective_count(bin, threshold);
        }
        if total <= 0.0 {
            return 0.0;
        }
        let hi = self.gated_percentile(threshold, total, 0.95);
        let lo = self.gated_percentile(threshold, total, 0.10);
        (hi - lo) as f32
    }

    /// How many of `bin`'s blocks survive the relative gate. Whole
    /// buckets strictly above / below `threshold` count fully / not at
    /// all; the one bucket the threshold cuts through contributes the
    /// fraction of its span above the threshold (blocks assumed uniform
    /// within a bucket).
    fn effective_count(&self, bin: usize, threshold: f64) -> f64 {
        let count = self.counts[bin];
        if count == 0 {
            return 0.0;
        }
        let lo = HIST_MIN_LUFS + bin as f64 * HIST_BIN_LU;
        let hi = lo + HIST_BIN_LU;
        if hi <= threshold {
            return 0.0;
        }
        if lo >= threshold {
            return count as f64;
        }
        count as f64 * (hi - threshold) / HIST_BIN_LU
    }

    /// Percentile of the relative-gated distribution, using the same
    /// `pct * (n - 1)` rank convention as a sorted-slice percentile. The
    /// value read back for a rank is its bucket's exact mean loudness
    /// (clamped to the gate), so quantisation error is bounded by one
    /// bucket width.
    fn gated_percentile(&self, threshold: f64, total: f64, pct: f64) -> f64 {
        let pos = pct * (total - 1.0).max(0.0);
        let mut cum = 0.0_f64;
        let mut last = threshold;
        for bin in 0..HIST_BINS {
            let count = self.effective_count(bin, threshold);
            if count <= 0.0 {
                continue;
            }
            let mean = (self.lufs_sums[bin] / self.counts[bin] as f64).max(threshold);
            if pos < cum + count {
                return mean;
            }
            cum += count;
            last = mean;
        }
        // Float round-off can leave the top rank a hair past the final
        // cumulative sum; it belongs to the last populated bucket.
        last
    }
}

impl Default for LraMeter {
    fn default() -> Self {
        Self::new()
    }
}

/// Histogram bucket for a block loudness, clamping out-of-range values
/// into the edge buckets.
fn bin_index(lufs: f64) -> usize {
    let clamped = lufs.clamp(HIST_MIN_LUFS, HIST_MAX_LUFS);
    (((clamped - HIST_MIN_LUFS) / HIST_BIN_LU) as usize).min(HIST_BINS - 1)
}
