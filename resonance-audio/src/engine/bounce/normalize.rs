//! Loudness measurement + gain-trim computation for the two-pass
//! normalized export path (doc #196, ba todo #652).
//!
//! The encoder pipeline produces interleaved stereo `f32` frames; the
//! normalization passes consume those same frames. [`LoudnessMeasure`]
//! wraps the BS.1770-4 meters from `resonance-metering` (integrated LUFS
//! plus 4× oversampled true peak) and accepts interleaved input, so both
//! the analyze pass and the post-limiter re-measure feed it directly.
//! [`target_gain_db`] turns a measurement into the gain trim for a
//! [`NormalizeSpec`].

use resonance_metering::{LufsMeter, TruePeakMeter};

use crate::types::{NormalizeMode, NormalizeSpec};

/// Integrated loudness + true peak of a rendered mix.
#[derive(Debug, Clone, Copy)]
pub(super) struct Loudness {
    /// Gated integrated loudness in LUFS, or `NEG_INFINITY` for silence.
    pub integrated_lufs: f32,
    /// Maximum true peak in dBTP (floored at `-120` for silence).
    pub true_peak_dbtp: f32,
}

/// Streaming integrated-LUFS + true-peak meter that accepts interleaved
/// stereo `f32`. Reused for the analyze pass and the post-limiter
/// re-measure.
pub(super) struct LoudnessMeasure {
    lufs: LufsMeter,
    true_peak: TruePeakMeter,
    /// Reusable de-interleave scratch so feeding a chunk allocates nothing.
    left: Vec<f32>,
    right: Vec<f32>,
}

impl LoudnessMeasure {
    pub(super) fn new(sample_rate: u32) -> Self {
        Self {
            lufs: LufsMeter::new(sample_rate as f32),
            true_peak: TruePeakMeter::new(),
            left: Vec::new(),
            right: Vec::new(),
        }
    }

    /// Feed one interleaved stereo chunk into both meters.
    pub(super) fn push(&mut self, frames: &[f32]) {
        let n = frames.len() / 2;
        self.left.clear();
        self.right.clear();
        self.left.reserve(n);
        self.right.reserve(n);
        for f in frames.chunks_exact(2) {
            self.left.push(f[0]);
            self.right.push(f[1]);
        }
        self.lufs.push_stereo(&self.left, &self.right);
        self.true_peak.push_stereo(&self.left, &self.right);
    }

    /// Read the accumulated loudness.
    pub(super) fn finish(&self) -> Loudness {
        Loudness {
            integrated_lufs: self.lufs.integrated_lufs(),
            true_peak_dbtp: self.true_peak.peak_dbtp(),
        }
    }
}

/// The gain trim (in dB) that moves `measured` toward `spec.target_db`.
/// `IntegratedLufs` trims by integrated loudness, `TruePeak` by measured
/// dBTP. A non-finite measurement (a silent project) yields `0.0` — there
/// is nothing to normalize and a finite trim keeps silence silent.
pub(super) fn target_gain_db(spec: &NormalizeSpec, measured: &Loudness) -> f32 {
    let measured_db = match spec.mode {
        NormalizeMode::IntegratedLufs => measured.integrated_lufs,
        NormalizeMode::TruePeak => measured.true_peak_dbtp,
    };
    if measured_db.is_finite() {
        spec.target_db - measured_db
    } else {
        0.0
    }
}
