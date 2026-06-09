//! LUFS meter facade.
//!
//! Owns per-channel [`KWeightingFilter`][crate::k_weighting::KWeightingFilter]
//! instances, a [`BlockAccumulator`] for sliding-window mean-squares, and
//! an [`IntegratedAccumulator`] for gated integrated loudness.
//!
//! Supports both streaming use (the mastering plugin's live meter) and
//! one-shot analysis (the future master assistant's 10 s capture) via
//! [`LufsMeter::analyze_offline`], which simply plays a buffer into a
//! fresh meter and reads the final state.

pub mod block_accumulator;
pub mod gating;
pub mod integrated;

use crate::k_weighting::KWeightingFilter;
use block_accumulator::BlockAccumulator;
use gating::block_mean_square_to_lufs;
use integrated::IntegratedAccumulator;

/// Aggregate of LUFS readouts computed by [`LufsMeter::analyze_offline`].
#[derive(Debug, Clone, Copy)]
pub struct LufsReadout {
    pub momentary: f32,
    pub short_term: f32,
    pub integrated: f32,
}

/// Streaming + one-shot LUFS meter.
pub struct LufsMeter {
    sample_rate: f32,
    k_left: KWeightingFilter,
    k_right: KWeightingFilter,
    block_acc: BlockAccumulator,
    integrated: IntegratedAccumulator,
}

impl LufsMeter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            k_left: KWeightingFilter::new(sample_rate),
            k_right: KWeightingFilter::new(sample_rate),
            block_acc: BlockAccumulator::new(sample_rate),
            integrated: IntegratedAccumulator::new(),
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn reset(&mut self) {
        self.k_left.reset();
        self.k_right.reset();
        self.block_acc.reset();
        self.integrated.reset();
    }

    /// Feed a stereo block. Zero allocations on the audio thread.
    pub fn push_stereo(&mut self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len());
        for i in 0..n {
            let kl = self.k_left.process(left[i]);
            let kr = self.k_right.process(right[i]);
            // BS.1770-4 stereo: sum of K-weighted channel squares with
            // unit weights (G_L = G_R = 1.0 for Lo/Ro).
            let sq = (kl as f64) * (kl as f64) + (kr as f64) * (kr as f64);
            if let Some(block_ms) = self.block_acc.push_sample(sq) {
                self.integrated.push_block(block_ms);
            }
        }
    }

    /// Momentary loudness (400 ms window). Returns `f32::NEG_INFINITY`
    /// if the window has not been populated yet.
    pub fn momentary_lufs(&self) -> f32 {
        match self.block_acc.momentary_mean_square() {
            Some(ms) => block_mean_square_to_lufs(ms) as f32,
            None => f32::NEG_INFINITY,
        }
    }

    /// Short-term loudness (3 s window).
    pub fn short_term_lufs(&self) -> f32 {
        match self.block_acc.short_term_mean_square() {
            Some(ms) => block_mean_square_to_lufs(ms) as f32,
            None => f32::NEG_INFINITY,
        }
    }

    /// Gated integrated loudness from the start of the current session
    /// (the last call to [`LufsMeter::reset`], or construction).
    pub fn integrated_lufs(&self) -> f32 {
        let v = self.integrated.integrated_lufs();
        if v.is_finite() {
            v as f32
        } else {
            f32::NEG_INFINITY
        }
    }

    /// Number of integrated-gating blocks currently held. Exposed for
    /// tests and diagnostics.
    pub fn integrated_block_count(&self) -> usize {
        self.integrated.len()
    }

    /// One-shot analysis of a stereo buffer. Runs a fresh meter end-to-end
    /// and returns all three LUFS readouts.
    pub fn analyze_offline(sample_rate: f32, left: &[f32], right: &[f32]) -> LufsReadout {
        let mut m = Self::new(sample_rate);
        m.push_stereo(left, right);
        LufsReadout {
            momentary: m.momentary_lufs(),
            short_term: m.short_term_lufs(),
            integrated: m.integrated_lufs(),
        }
    }
}

/// Reference implementations may need the BS.1770-4 loudness offset.
pub use gating::LOUDNESS_OFFSET as BS1770_LOUDNESS_OFFSET;
