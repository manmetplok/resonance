//! Per-channel 4× oversampled peak detector.
//!
//! Direct FIR convolution — at 48 taps the cost is trivial (a handful of
//! multiply-adds per input sample per channel) so there is no reason to
//! use FFT-based upsampling.

use super::coefficients::{FIR, PHASES, TAPS};

/// Streaming oversampled peak detector for one audio channel.
pub struct PolyphasePeakDetector {
    /// History of the last `TAPS` input samples, most recent at
    /// `write_pos - 1` (mod `TAPS`).
    history: [f32; TAPS],
    write_pos: usize,
    /// Running max-abs across the oversampled stream since the last
    /// [`Self::reset_peak`] call.
    peak: f32,
}

impl PolyphasePeakDetector {
    pub const fn new() -> Self {
        Self {
            history: [0.0; TAPS],
            write_pos: 0,
            peak: 0.0,
        }
    }

    /// Clear the filter's history and reset the held peak.
    pub fn reset(&mut self) {
        self.history = [0.0; TAPS];
        self.write_pos = 0;
        self.peak = 0.0;
    }

    /// Reset the held peak without touching the filter history.
    pub fn reset_peak(&mut self) {
        self.peak = 0.0;
    }

    /// Current held peak (linear magnitude).
    pub fn peak(&self) -> f32 {
        self.peak
    }

    /// Process one input sample. Produces four oversampled output samples
    /// internally and updates the held peak to the maximum |x| seen.
    #[inline]
    pub fn push_sample(&mut self, sample: f32) {
        self.history[self.write_pos] = sample;
        self.write_pos = (self.write_pos + 1) % TAPS;

        // Also account for the input sample itself — at discrete-time
        // indices the original sample IS an oversampled output (at phase 0
        // with non-unity coefficient, but the DC gain of the filter is 1,
        // so tracking input directly is a cheap conservative peak floor).
        let abs_in = sample.abs();
        if abs_in > self.peak {
            self.peak = abs_in;
        }

        // Convolve against each polyphase sub-filter. `j = 0` multiplies
        // the most recent sample, i.e. the one we just wrote.
        for p in 0..PHASES {
            let mut acc = 0.0_f32;
            let taps = &FIR[p];
            for j in 0..TAPS {
                let idx = (self.write_pos + TAPS - 1 - j) % TAPS;
                acc += taps[j] * self.history[idx];
            }
            let abs = acc.abs();
            if abs > self.peak {
                self.peak = abs;
            }
        }
    }

    /// Process a slice of samples.
    pub fn push_block(&mut self, samples: &[f32]) {
        for &s in samples {
            self.push_sample(s);
        }
    }
}

impl Default for PolyphasePeakDetector {
    fn default() -> Self {
        Self::new()
    }
}
