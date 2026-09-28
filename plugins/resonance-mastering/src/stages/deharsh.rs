//! De-harsh stage: `resonance_dsp`'s [`ResonanceSuppressor`] between the
//! corrective EQ and the glue compressor
//! (`docs/design/deharsh-resonance-suppressor.md`).
//!
//! The suppressor delays by one STFT frame ([`DeharshStage::latency`],
//! 2048 samples at 48 kHz), and the chain reports that on top of its
//! other stages in every state (a plugin cannot move its latency without
//! a restart, F2). Off is the suppressor's own delay tap, bit-exact, and
//! on/off crossfade over 10 ms. The STFT keeps running while off, so
//! switching on lands on a warm path.

use resonance_dsp::{ResonanceSuppressor, SuppressorConfig};

pub struct DeharshStage {
    sup: ResonanceSuppressor,
}

impl DeharshStage {
    pub fn new(sample_rate: f32) -> Self {
        let mut sup = ResonanceSuppressor::new(sample_rate);
        let hop = sup.geometry().hop;
        // FFT stagger (DSP-16): run the frames at 3/4 of each hop. With
        // a 128-frame quantum at 48 kHz that is the odd callbacks, which
        // hold 4 of the 10 primary convolver slots and 4 of the 10 M/S
        // cross slots (the even ones hold 6 + 6). At a hop of 256 every
        // other callback runs a frame, so no phase avoids them all.
        sup.set_phase_offset(hop / 4 - 1);
        Self { sup }
    }

    /// Constant latency in samples, whatever the config.
    pub fn latency(&self) -> usize {
        self.sup.latency()
    }

    /// Deepest current cut, dB (for meters).
    pub fn max_cut_db(&self) -> f32 {
        self.sup.max_cut_db()
    }

    pub fn reset(&mut self) {
        self.sup.reset();
    }

    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32], cfg: &SuppressorConfig) {
        self.sup.process_stereo(left, right, cfg);
    }
}
