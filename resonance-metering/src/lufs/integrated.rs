//! Streaming integrated-loudness accumulator.
//!
//! Holds the growing list of per-block mean-square energies produced by the
//! [`BlockAccumulator`][crate::lufs::block_accumulator::BlockAccumulator]
//! and runs the BS.1770-4 two-pass gate on demand.
//!
//! The Vec is pre-grown to the capacity implied by a maximum-session
//! length so the audio thread never reallocates during normal operation.
//! Pushing past the cap is a hard error in debug builds and a silently
//! dropped block in release builds — both are acceptable for the 60-minute
//! cap we use today.

use super::block_accumulator::BLOCK_HOP_SECS;
use super::gating::gated_integrated_lufs;

/// Maximum number of seconds of audio the integrated meter can hold before
/// it starts dropping new blocks. Pick something generous enough to cover
/// any realistic mastering session.
pub const MAX_SESSION_SECONDS: f32 = 60.0 * 60.0;

/// Accumulator for per-block mean-squares used by the integrated LUFS
/// calculation. Pure data container + one reader function.
pub struct IntegratedAccumulator {
    blocks: Vec<f64>,
    /// Soft cap on the number of blocks we accept before dropping new ones.
    cap: usize,
    /// How many blocks were dropped after the cap was reached. Exposed so
    /// callers can report overflow in the UI / test harness.
    dropped: u64,
}

impl IntegratedAccumulator {
    pub fn new() -> Self {
        let cap = (MAX_SESSION_SECONDS / BLOCK_HOP_SECS).ceil() as usize;
        Self {
            blocks: Vec::with_capacity(cap),
            cap,
            dropped: 0,
        }
    }

    pub fn reset(&mut self) {
        self.blocks.clear();
        self.dropped = 0;
    }

    /// Add one block mean-square. If the cap has been reached, the value
    /// is dropped and `dropped_blocks()` is incremented.
    #[inline]
    pub fn push_block(&mut self, mean_square: f64) {
        if self.blocks.len() < self.cap {
            self.blocks.push(mean_square);
        } else {
            self.dropped += 1;
            debug_assert!(
                false,
                "IntegratedAccumulator exceeded cap of {} blocks ({:.0} min)",
                self.cap,
                MAX_SESSION_SECONDS / 60.0
            );
        }
    }

    /// Number of blocks currently held.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether the accumulator has any blocks yet.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Number of blocks dropped after hitting the cap.
    pub fn dropped_blocks(&self) -> u64 {
        self.dropped
    }

    /// Run the two-pass gate and return the integrated LUFS value. Returns
    /// `f64::NEG_INFINITY` if there's nothing to report yet.
    pub fn integrated_lufs(&self) -> f64 {
        gated_integrated_lufs(&self.blocks)
    }
}

impl Default for IntegratedAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lufs::gating::{block_mean_square_to_lufs, LOUDNESS_OFFSET};

    fn lufs_to_ms(lufs: f64) -> f64 {
        10.0_f64.powf((lufs - LOUDNESS_OFFSET) / 10.0)
    }

    #[test]
    fn new_accumulator_reports_neg_infinity() {
        let acc = IntegratedAccumulator::new();
        assert!(acc.integrated_lufs().is_infinite());
    }

    #[test]
    fn pushes_and_gates_produce_expected_loudness() {
        let mut acc = IntegratedAccumulator::new();
        for _ in 0..100 {
            acc.push_block(lufs_to_ms(-20.0));
        }
        let got = acc.integrated_lufs();
        assert!((got - -20.0).abs() < 1e-6, "got {got}");
    }

    #[test]
    fn reset_clears_all_state() {
        let mut acc = IntegratedAccumulator::new();
        for _ in 0..10 {
            acc.push_block(1.0);
        }
        acc.reset();
        assert_eq!(acc.len(), 0);
        assert!(acc.integrated_lufs().is_infinite());
    }

    #[test]
    fn block_ms_round_trip() {
        for lufs in [-70.0, -40.0, -23.0, -14.0, 0.0] {
            let ms = lufs_to_ms(lufs);
            let back = block_mean_square_to_lufs(ms);
            assert!((back - lufs).abs() < 1e-10);
        }
    }
}
