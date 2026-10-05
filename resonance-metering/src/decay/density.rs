//! Abel & Huang normalised echo density (AES 121st Convention, 2006).
//!
//! For a window `w` centred on `t`, with `σ` the window-weighted RMS of the
//! response under it:
//!
//! ```text
//! η(t) = (1 / erfc(1/√2)) · Σ w(τ) · 1{|h(t+τ)| > σ}     (Σ w = 1)
//! ```
//!
//! Gaussian noise has `erfc(1/√2) ≈ 0.3173` of its samples outside one
//! standard deviation, so `η ≈ 1` once the response is as dense as noise,
//! and `η ≪ 1` while it is still a train of separate reflections.
//!
//! The window is a Hann window. Near the start and end it is cut to the
//! samples that exist and renormalised, so `η` is defined from `t = 0`.
//! Times are seconds from the start of the analysed buffer (for a plugin
//! render, from the excitation), not from an onset.

/// Window length Abel & Huang recommend, seconds.
pub const ECHO_DENSITY_WINDOW_S: f32 = 0.020;
/// Profile hop, seconds.
pub const ECHO_DENSITY_HOP_S: f32 = 0.001;

/// `erfc(1/√2)`: the fraction of a Gaussian outside ±1σ.
const ERFC_INV_SQRT2: f64 = 0.317_310_507_862_914_1;

/// Echo density sampled every [`EchoDensity::hop_s`] seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct EchoDensity {
    /// Time between successive values, seconds. `values[k]` is the window
    /// centred on `k · hop_s`.
    pub hop_s: f32,
    pub values: Vec<f32>,
}

impl EchoDensity {
    /// First time the profile reaches `threshold` (`1.0` = Gaussian
    /// density; `0.9` is a steadier choice for responses that settle just
    /// under 1), seconds. `None` if it never does.
    pub fn time_to_reach(&self, threshold: f32) -> Option<f32> {
        self.values
            .iter()
            .position(|&v| v >= threshold)
            .map(|k| k as f32 * self.hop_s)
    }

    /// Mean of the profile over `[from_s, to_s)`, or `None` if empty.
    pub fn mean_between(&self, from_s: f32, to_s: f32) -> Option<f32> {
        let a = (from_s / self.hop_s).ceil().max(0.0) as usize;
        let b = ((to_s / self.hop_s).ceil() as usize).min(self.values.len());
        if a >= b {
            return None;
        }
        Some(self.values[a..b].iter().sum::<f32>() / (b - a) as f32)
    }
}

/// Echo density profile of `ir` with a `window_s` Hann window and a
/// [`ECHO_DENSITY_HOP_S`] hop. See the module docs.
pub fn echo_density_profile(ir: &[f32], sample_rate: f32, window_s: f32) -> EchoDensity {
    let hop = ((ECHO_DENSITY_HOP_S * sample_rate).round() as usize).max(1);
    let len = (((window_s * sample_rate).round() as usize) | 1).max(3);
    let half = len / 2;
    let window: Vec<f64> = (0..len)
        .map(|k| {
            let x = (k as f64 + 1.0) / (len as f64 + 1.0);
            0.5 - 0.5 * (std::f64::consts::TAU * x).cos()
        })
        .collect();

    let n = ir.len();
    let mut values = Vec::with_capacity(n / hop + 1);
    let mut centre = 0usize;
    while centre < n {
        let first = centre.saturating_sub(half);
        let last = (centre + half + 1).min(n);
        let w_off = first + half - centre;
        let (mut sw, mut swe) = (0.0f64, 0.0f64);
        for (i, &x) in ir[first..last].iter().enumerate() {
            let w = window[w_off + i];
            sw += w;
            swe += w * (x as f64) * (x as f64);
        }
        let eta = if swe > 0.0 && sw > 0.0 {
            let sigma = (swe / sw).sqrt();
            let mut outside = 0.0f64;
            for (i, &x) in ir[first..last].iter().enumerate() {
                if (x as f64).abs() > sigma {
                    outside += window[w_off + i];
                }
            }
            (outside / sw / ERFC_INV_SQRT2) as f32
        } else {
            0.0
        };
        values.push(eta);
        centre += hop;
    }
    EchoDensity {
        hop_s: hop as f32 / sample_rate,
        values,
    }
}
