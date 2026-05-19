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
//! We capture a new block roughly every 1 s via the supplied hook. The
//! percentile step is performed on demand from `lra_lu()` so the audio
//! thread never sorts.

use crate::lufs::gating::{block_mean_square_to_lufs, ABSOLUTE_GATE_LUFS};

/// Relative gate offset used by the LRA calculation (distinct from the
/// -10 LU relative gate used for integrated loudness).
pub const LRA_RELATIVE_GATE_LU: f64 = -20.0;

/// Hard cap on the number of 1 s blocks we hold before dropping new ones.
const BLOCK_CAP: usize = 60 * 60; // 60 minutes of 1 s blocks.

/// Streaming LRA tracker.
pub struct LraMeter {
    blocks: Vec<f64>, // mean-square per 3 s block (recorded every 1 s)
    dropped: u64,
}

impl LraMeter {
    pub fn new() -> Self {
        Self {
            blocks: Vec::with_capacity(BLOCK_CAP),
            dropped: 0,
        }
    }

    pub fn reset(&mut self) {
        self.blocks.clear();
        self.dropped = 0;
    }

    /// Record a 3-second short-term mean-square. Intended to be called at
    /// ~1 Hz from the LUFS meter's host (see `LufsMeter`).
    pub fn push_short_term_mean_square(&mut self, mean_square: f64) {
        if self.blocks.len() < BLOCK_CAP {
            self.blocks.push(mean_square);
        } else {
            self.dropped += 1;
        }
    }

    /// Compute LRA in LU. Returns 0.0 for an empty / silent session so
    /// the UI has a sane default.
    pub fn lra_lu(&self) -> f32 {
        if self.blocks.is_empty() {
            return 0.0;
        }
        // Absolute gate + accumulate energetic mean for the relative-gate
        // reference. Per EBU 3342 the relative gate threshold is
        // `integrated_loudness(abs_gated) - 20 LU`, where the integrated
        // loudness is the LUFS of the *mean of mean-squares* — not a
        // percentile of the per-block LUFS values.
        let mut abs_sum_ms = 0.0_f64;
        let mut abs_lufs: Vec<f64> = Vec::with_capacity(self.blocks.len());
        for &ms in &self.blocks {
            let l = block_mean_square_to_lufs(ms);
            if l >= ABSOLUTE_GATE_LUFS {
                abs_sum_ms += ms;
                abs_lufs.push(l);
            }
        }
        if abs_lufs.is_empty() {
            return 0.0;
        }
        let reference_lufs = block_mean_square_to_lufs(abs_sum_ms / abs_lufs.len() as f64);
        let threshold = reference_lufs + LRA_RELATIVE_GATE_LU;

        let mut gated: Vec<f64> = abs_lufs.into_iter().filter(|&l| l >= threshold).collect();
        if gated.is_empty() {
            return 0.0;
        }
        gated.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let hi = percentile(&gated, 0.95);
        let lo = percentile(&gated, 0.10);
        (hi - lo) as f32
    }
}

impl Default for LraMeter {
    fn default() -> Self {
        Self::new()
    }
}

/// Linear-interpolated percentile of a **sorted** slice.
fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let pos = pct * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(sorted.len() - 1);
    let frac = pos - lo as f64;
    sorted[lo] + (sorted[hi] - sorted[lo]) * frac
}

