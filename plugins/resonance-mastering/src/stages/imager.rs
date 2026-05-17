//! M/S stereo imager.
//!
//! Encodes L/R to Mid/Side, optionally high-pass-filters the side
//! channel (keeps sub-bass mono for vinyl and phone-speaker
//! compatibility), scales the side channel by `width`, and decodes
//! back to L/R. Zero latency — a single biquad per call path.
//!
//! `width == 1.0` is the identity. `width == 0.0` collapses to mono.
//! `width > 1.0` widens the image (caution: can cause mono-sum
//! cancellation). The recommended safe range for mastering band
//! material is `0.8 .. 1.3`.

use resonance_dsp::Biquad;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImagerConfig {
    pub enabled: bool,
    /// Side-channel gain. 0 = mono, 1 = unchanged, 2 = doubled side.
    pub width: f32,
    /// Apply a high-pass to the side channel before width scaling?
    pub side_hpf_on: bool,
    /// Side HPF cutoff frequency.
    pub side_hpf_hz: f32,
}

impl Default for ImagerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            width: 1.0,
            side_hpf_on: false,
            side_hpf_hz: 120.0,
        }
    }
}

pub struct Imager {
    sample_rate: f32,
    side_hpf: Biquad,
    /// Last cutoff used to configure the HPF — avoids recomputing
    /// biquad coefficients every block when the param hasn't moved.
    cached_hpf_hz: f32,
}

impl Imager {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            side_hpf: Biquad::identity(),
            cached_hpf_hz: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.side_hpf.reset();
    }

    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32], cfg: &ImagerConfig) {
        if !cfg.enabled {
            return;
        }

        if cfg.side_hpf_on && (self.cached_hpf_hz - cfg.side_hpf_hz).abs() > 0.5 {
            self.side_hpf
                .set_high_pass(self.sample_rate, cfg.side_hpf_hz.max(20.0), 0.707);
            self.cached_hpf_hz = cfg.side_hpf_hz;
        }

        let width = cfg.width.clamp(0.0, 2.0);
        let frames = left.len().min(right.len());
        for i in 0..frames {
            let l = left[i];
            let r = right[i];
            let mid = 0.5 * (l + r);
            let mut side = 0.5 * (l - r);
            if cfg.side_hpf_on {
                side = self.side_hpf.process(side);
            }
            side *= width;
            left[i] = mid + side;
            right[i] = mid - side;
        }
    }
}

