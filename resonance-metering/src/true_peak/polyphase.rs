//! Per-channel 4× oversampled peak detector.
//!
//! Direct FIR convolution — at 48 taps the cost is trivial (a handful of
//! multiply-adds per input sample per channel) so there is no reason to
//! use FFT-based upsampling.

use super::coefficients::{FIR, PHASES, TAPS};

/// Streaming oversampled peak detector for one audio channel.
pub struct PolyphasePeakDetector {
    /// Mirrored history of the last `TAPS` input samples: every sample is
    /// written at `write_pos` *and* `write_pos + TAPS`, so for any
    /// `p = write_pos` in `0..TAPS` the slice `history[p + 1..=p + TAPS]`
    /// holds the last `TAPS` samples in arrival order (most recent last).
    /// This keeps the convolution loop free of per-tap `%` index math.
    history: [f32; 2 * TAPS],
    write_pos: usize,
    /// Running max-abs across the oversampled stream since the last
    /// [`Self::reset_peak`] call.
    peak: f32,
}

impl PolyphasePeakDetector {
    pub const fn new() -> Self {
        Self {
            history: [0.0; 2 * TAPS],
            write_pos: 0,
            peak: 0.0,
        }
    }

    /// Clear the filter's history and reset the held peak.
    pub fn reset(&mut self) {
        self.history = [0.0; 2 * TAPS];
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
        let p = self.write_pos;
        // Mirror the write so `history[p + 1..=p + TAPS]` is always a
        // contiguous view of the last TAPS samples, oldest first.
        self.history[p] = sample;
        self.history[p + TAPS] = sample;
        self.write_pos = if p + 1 == TAPS { 0 } else { p + 1 };

        // Also account for the input sample itself — at discrete-time
        // indices the original sample IS an oversampled output (at phase 0
        // with non-unity coefficient, but the DC gain of the filter is 1,
        // so tracking input directly is a cheap conservative peak floor).
        let abs_in = sample.abs();
        if abs_in > self.peak {
            self.peak = abs_in;
        }

        // Convolve against each polyphase sub-filter. `taps[0]` multiplies
        // the most recent sample (the one we just wrote), so pair the taps
        // with the linear window walked newest-to-oldest — no modulo in
        // the inner loop.
        let window = &self.history[p + 1..p + 1 + TAPS];
        for taps in FIR.iter().take(PHASES) {
            let mut acc = 0.0_f32;
            for (&tap, &x) in taps.iter().zip(window.iter().rev()) {
                acc += tap * x;
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
