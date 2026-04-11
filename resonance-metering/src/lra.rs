//! Loudness Range (LRA) per EBU Tech 3342.
//!
//! The LRA is computed from the distribution of 3-second ungated short-
//! term loudness measurements across a session:
//!
//! 1. Accumulate short-term blocks every 1 s (3 s window, 2 s overlap).
//! 2. Absolute-gate at -70 LUFS.
//! 3. Relative-gate at the 95th percentile minus 20 LU of the absolute-
//!    gated set (the EBU 3342 variant uses -20 LU rather than the -10 LU
//!    used by integrated).
//! 4. Report LRA = p95 - p10 of the remaining set.
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
        // Absolute gate.
        let abs_gated: Vec<f64> = self
            .blocks
            .iter()
            .copied()
            .filter(|&ms| block_mean_square_to_lufs(ms) >= ABSOLUTE_GATE_LUFS)
            .collect();
        if abs_gated.is_empty() {
            return 0.0;
        }
        // Compute the p95 reference from the absolute-gated set, then
        // relative-gate at `p95 + LRA_RELATIVE_GATE_LU`.
        let mut abs_lufs: Vec<f64> = abs_gated.iter().map(|&ms| block_mean_square_to_lufs(ms)).collect();
        abs_lufs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p95 = percentile(&abs_lufs, 0.95);
        let threshold = p95 + LRA_RELATIVE_GATE_LU;

        let mut gated: Vec<f64> = abs_lufs
            .into_iter()
            .filter(|&l| l >= threshold)
            .collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lufs::gating::LOUDNESS_OFFSET;

    fn lufs_to_ms(lufs: f64) -> f64 {
        10.0_f64.powf((lufs - LOUDNESS_OFFSET) / 10.0)
    }

    #[test]
    fn empty_session_reports_zero_lra() {
        let lra = LraMeter::new();
        assert_eq!(lra.lra_lu(), 0.0);
    }

    #[test]
    fn constant_level_yields_near_zero_lra() {
        let mut lra = LraMeter::new();
        for _ in 0..100 {
            lra.push_short_term_mean_square(lufs_to_ms(-20.0));
        }
        assert!(lra.lra_lu().abs() < 0.1);
    }

    #[test]
    fn step_from_quiet_to_loud_has_lra_near_the_step() {
        // Step sequence: 20→30→20 dBFS input levels (which map to different
        // LUFS values). This exercises the percentile calculation.
        let mut lra = LraMeter::new();
        for _ in 0..10 {
            lra.push_short_term_mean_square(lufs_to_ms(-20.0));
        }
        for _ in 0..10 {
            lra.push_short_term_mean_square(lufs_to_ms(-30.0));
        }
        for _ in 0..10 {
            lra.push_short_term_mean_square(lufs_to_ms(-20.0));
        }
        let v = lra.lra_lu();
        // Expected LRA ≈ 10 LU (the step height); allow a generous band.
        assert!(v > 5.0 && v < 15.0, "LRA = {v}");
    }
}
