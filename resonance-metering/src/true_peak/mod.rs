//! Stereo true-peak meter per ITU-R BS.1770-4 Annex 2.
//!
//! Each channel runs through its own 4× polyphase oversampler and the
//! held peaks are reported independently. dBTP is computed via the usual
//! `20*log10(linear_peak)` with a floor to avoid `-inf`.

pub mod coefficients;
pub mod polyphase;

use polyphase::PolyphasePeakDetector;

/// Minimum dBTP value reported when the detector is silent.
pub const FLOOR_DBTP: f32 = -120.0;

/// Streaming stereo true-peak meter.
pub struct TruePeakMeter {
    left: PolyphasePeakDetector,
    right: PolyphasePeakDetector,
}

impl TruePeakMeter {
    pub fn new() -> Self {
        Self {
            left: PolyphasePeakDetector::new(),
            right: PolyphasePeakDetector::new(),
        }
    }

    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }

    /// Reset only the held peak values without clearing filter history.
    /// Use this between measurement windows.
    pub fn reset_peak(&mut self) {
        self.left.reset_peak();
        self.right.reset_peak();
    }

    /// Feed a stereo block.
    #[inline]
    pub fn push_stereo(&mut self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len());
        for i in 0..n {
            self.left.push_sample(left[i]);
            self.right.push_sample(right[i]);
        }
    }

    /// Max-abs true peak across both channels, linear magnitude.
    pub fn peak_linear(&self) -> f32 {
        self.left.peak().max(self.right.peak())
    }

    /// Max-abs true peak across both channels, in dBTP.
    pub fn peak_dbtp(&self) -> f32 {
        linear_to_dbtp(self.peak_linear())
    }

    /// Per-channel true peaks in dBTP.
    pub fn per_channel_dbtp(&self) -> (f32, f32) {
        (
            linear_to_dbtp(self.left.peak()),
            linear_to_dbtp(self.right.peak()),
        )
    }
}

impl Default for TruePeakMeter {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn linear_to_dbtp(linear: f32) -> f32 {
    if linear > 0.0 {
        20.0 * linear.log10()
    } else {
        FLOOR_DBTP
    }
}

