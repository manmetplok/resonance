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

    pub fn process_stereo(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        cfg: &ImagerConfig,
    ) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_passes_audio_unchanged() {
        let mut im = Imager::new(48_000.0);
        let mut l = vec![0.3, -0.4, 0.5, -0.6];
        let mut r = vec![0.2, -0.3, 0.4, -0.5];
        let el = l.clone();
        let er = r.clone();
        im.process_stereo(&mut l, &mut r, &ImagerConfig::default());
        assert_eq!(l, el);
        assert_eq!(r, er);
    }

    #[test]
    fn width_one_is_identity() {
        let mut im = Imager::new(48_000.0);
        let mut l = vec![0.3_f32, -0.4, 0.5, -0.6];
        let mut r = vec![0.2_f32, -0.3, 0.4, -0.5];
        let el = l.clone();
        let er = r.clone();
        im.process_stereo(
            &mut l,
            &mut r,
            &ImagerConfig {
                enabled: true,
                width: 1.0,
                side_hpf_on: false,
                side_hpf_hz: 120.0,
            },
        );
        for (a, b) in l.iter().zip(el.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
        for (a, b) in r.iter().zip(er.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn width_zero_collapses_to_mono() {
        let mut im = Imager::new(48_000.0);
        // L and R start different but should both become 0.5*(L+R).
        let mut l = vec![0.4_f32, -0.6, 0.8, 0.0];
        let mut r = vec![0.0_f32, 0.0, -0.2, 0.4];
        let expected_mono: Vec<f32> = l.iter().zip(r.iter()).map(|(a, b)| 0.5 * (a + b)).collect();
        im.process_stereo(
            &mut l,
            &mut r,
            &ImagerConfig {
                enabled: true,
                width: 0.0,
                side_hpf_on: false,
                side_hpf_hz: 120.0,
            },
        );
        for (i, (a, b)) in l.iter().zip(expected_mono.iter()).enumerate() {
            assert!((a - b).abs() < 1e-6, "left[{i}] {a} vs {b}");
        }
        for (i, (a, b)) in r.iter().zip(expected_mono.iter()).enumerate() {
            assert!((a - b).abs() < 1e-6, "right[{i}] {a} vs {b}");
        }
    }

    #[test]
    fn side_hpf_removes_low_frequencies_from_side_channel() {
        // Build a 50 Hz anti-phase signal (pure side content).
        // After side HPF at 200 Hz the side should be heavily
        // attenuated, so L and R converge to the mono sum (= 0).
        let sr = 48_000.0_f32;
        let mut im = Imager::new(sr);
        let n = 4096;
        let mut l = vec![0.0_f32; n];
        let mut r = vec![0.0_f32; n];
        for i in 0..n {
            let s = (i as f32 / sr * 50.0 * std::f32::consts::TAU).sin() * 0.5;
            l[i] = s;
            r[i] = -s;
        }
        im.process_stereo(
            &mut l,
            &mut r,
            &ImagerConfig {
                enabled: true,
                width: 1.0,
                side_hpf_on: true,
                side_hpf_hz: 200.0,
            },
        );
        // Look at the settled tail.
        let tail = &l[n / 2..];
        let peak = tail.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
        assert!(peak < 0.05, "low-freq side peak = {peak}");
    }
}
