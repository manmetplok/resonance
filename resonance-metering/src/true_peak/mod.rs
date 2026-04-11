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
        (linear_to_dbtp(self.left.peak()), linear_to_dbtp(self.right.peak()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dc_settles_near_zero_dbtp_after_warmup() {
        // The polyphase FIR has a startup transient — during the first
        // dozen samples after a 0→1 step, partial sums across some phases
        // exceed unity. Once the history is full the DC gain settles at
        // ~+0.014 dBTP (the max of the four per-phase signed tap sums).
        // Warm up, reset the held peak, then measure steady state.
        let mut m = TruePeakMeter::new();
        let warmup = [1.0_f32; 512];
        m.push_stereo(&warmup, &warmup);
        m.reset_peak();
        let steady = [1.0_f32; 128];
        m.push_stereo(&steady, &steady);
        let dbtp = m.peak_dbtp();
        assert!(
            dbtp.abs() < 0.1,
            "DC after warmup → {dbtp} dBTP (expected |·| < 0.1)"
        );
    }

    #[test]
    fn silence_floors_out() {
        let m = TruePeakMeter::new();
        assert_eq!(m.peak_dbtp(), FLOOR_DBTP);
    }

    #[test]
    fn detects_inter_sample_peak_above_sample_peak() {
        // A sine near Nyquist phase-aligned so discrete samples all miss
        // the true peak. Feed at 0.499*fs with a phase that makes samples
        // land symmetrically about 0, so the discrete peak ≈ cos(pi*0.499)
        // while the true peak is ~1.0.
        let sr = 48_000.0_f32;
        let f = 0.499 * sr; // just below Nyquist
        let n = 8192usize;
        let mut l = vec![0.0_f32; n];
        let mut r = vec![0.0_f32; n];
        for i in 0..n {
            // Offset by 0.5 so samples bracket but never hit the peak.
            let t = (i as f32 + 0.5) / sr;
            let s = (std::f32::consts::TAU * f * t).cos();
            l[i] = s;
            r[i] = s;
        }
        // Discrete sample peak.
        let discrete_peak = l.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
        let mut m = TruePeakMeter::new();
        m.push_stereo(&l, &r);
        let true_peak = m.peak_linear();
        // The oversampled true peak should recover more than the discrete
        // samples alone.
        assert!(true_peak >= discrete_peak,
            "true peak {} should be >= discrete {}", true_peak, discrete_peak);
        // And it should get close to 1.0 (within 1 dB).
        assert!(m.peak_dbtp() > -1.0,
            "expected near-unity true peak, got {} dBTP", m.peak_dbtp());
    }
}
