//! Per-line frequency-dependent absorption for feedback delay networks
//! (Jot & Chaigne, AES 1991).
//!
//! A delay line of `d` samples that recirculates must lose exactly 60 dB
//! every `T60` seconds, i.e. its loop gain per pass is
//!
//! ```text
//! g(T60) = 10^(−3·d / (T60·fs))
//! ```
//!
//! [`Absorption`] makes that gain frequency dependent with three bands:
//! `g_L` from `t60_low`, `g_M` from `t60_mid`, `g_H` from `t60_high`. It is
//! a broadband gain `g` times two first-order shelves:
//!
//! - a **low shelf** with DC gain `G_L = g_L / g` and unity at Nyquist,
//!   analog prototype `H(s) = (s + Ω·√G_L) / (s + Ω/√G_L)`;
//! - a **high shelf** with Nyquist gain `G_H = g_H / g` and unity at DC,
//!   analog prototype `H(s) = (√G_H·s + Ω) / (s/√G_H + Ω)`.
//!
//! Each prototype has its geometric midpoint `√G` at `s = jΩ`, so the
//! crossover frequency is where the shelf is "half way" in dB. They are
//! discretised by the bilinear transform with `Ω = tan(π·f_c/fs)` (the
//! crossover pre-warped), which maps `s = 0` to DC and `s = ∞` to Nyquist
//! exactly. So the loop gain is **exact** at DC (`g·G_L = g_L`) and at
//! Nyquist (`g·G_H = g_H`) whatever `g` is.
//!
//! **Mid compensation.** With `g = g_M` the mid band would carry the
//! shelves' skirts: for a small log-gain, `ln|H_low(Ω)| ≈ ln G_L /
//! (1 + (Ω/Ω_L)²)` (and the mirror for the high shelf), so with crossovers
//! 4× either side of the mid the mid T60 is off by up to ~10 % for strong
//! treble damping. Instead `g` is solved so that the loop gain is also
//! exact at the mid frequency `f_M = √(f_L·f_H)` (1 kHz for 250 Hz / 4 kHz):
//! in the log domain the mid gain is `ln g + α·ln G_L + β·ln G_H` with the
//! skirt weights `α = 1/(1 + (Ω_M/Ω_L)²)`, `β = r²/(1 + r²)`, `r = Ω_M/Ω_H`,
//! which gives
//!
//! ```text
//! ln g = (ln g_M − α·ln g_L − β·ln g_H) / (1 − α − β)
//! ```
//!
//! refined by three Newton steps on the exact digital magnitude (the
//! weights are only first-order accurate for very lossy lines). `g` may
//! exceed `g_M` (even `g_L`): with a strong treble cut the high shelf's
//! skirt has to be paid back. It is capped at 1. That keeps the loop
//! passive: each shelf's log-magnitude is a monotone fraction `w ∈ [0, 1]`
//! of its log-gain, so `ln|H| = (1 − w_L − w_H)·ln g + w_L·ln g_L +
//! w_H·ln g_H`, a convex combination of non-positive numbers while the
//! crossovers are apart (`w_L + w_H ≤ 1`). When they sit closer than about
//! 2.5× (`1 − α − β < 0.2`) the compensation is skipped and `g = g_M`.
//! What is left at 50 Hz and 16 kHz is the skirt residual, ~4 % of the band
//! T60 at a 5× distance from the crossover.
//!
//! First order keeps it to six multiplies and two states per line, and
//! its phase is benign (a fraction of a sample of group delay at most).

/// The three-band decay target an [`Absorption`] (and an `Fdn`) is
/// designed for. Times are T60 in seconds, crossovers in Hz.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecayBands {
    pub t60_low: f32,
    pub t60_mid: f32,
    pub t60_high: f32,
    /// Below this the decay tends to `t60_low`.
    pub low_xover_hz: f32,
    /// Above this the decay tends to `t60_high`.
    pub high_xover_hz: f32,
}

impl DecayBands {
    /// The same T60 at every frequency (both shelves flat).
    pub fn flat(t60: f32) -> Self {
        Self {
            t60_low: t60,
            t60_mid: t60,
            t60_high: t60,
            low_xover_hz: 250.0,
            high_xover_hz: 4000.0,
        }
    }

    /// The plugin's parameterisation: a mid T60 and bass/treble
    /// multipliers on it (`low_decay_mult`, `high_decay_mult`).
    pub fn from_mults(t60_mid: f32, low_mult: f32, high_mult: f32, low_xover_hz: f32, high_xover_hz: f32) -> Self {
        Self {
            t60_low: t60_mid * low_mult,
            t60_mid,
            t60_high: t60_mid * high_mult,
            low_xover_hz,
            high_xover_hz,
        }
    }
}

/// Loop gain per pass for a `delay_samples` line decaying 60 dB in `t60`
/// seconds: `10^(−3·d / (T60·fs))`. A non-positive or non-finite `t60`
/// reads as "no decay" only when it is `+∞`; anything else ≤ 0 is clamped
/// to 1 ms.
#[inline]
pub fn loop_gain(delay_samples: f32, t60: f32, sample_rate: f32) -> f32 {
    if t60 == f32::INFINITY {
        return 1.0;
    }
    let t60 = if t60.is_finite() { t60.max(1e-3) } else { 1e-3 };
    10f64.powf(-3.0 * delay_samples as f64 / (t60 as f64 * sample_rate as f64)) as f32
}

/// First-order section `y = b0·x + b1·x₋₁ − a1·y₋₁`, transposed DF-II.
#[derive(Clone, Copy, Debug)]
struct FirstOrder {
    b0: f32,
    b1: f32,
    a1: f32,
    z: f32,
}

impl FirstOrder {
    const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        a1: 0.0,
        z: 0.0,
    };

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z;
        self.z = self.b1 * x - self.a1 * y;
        y
    }

    fn magnitude(&self, w: f64) -> f64 {
        // H(e^{jw}) = (b0 + b1 e^{-jw}) / (1 + a1 e^{-jw})
        let (s, c) = w.sin_cos();
        let (b0, b1, a1) = (self.b0 as f64, self.b1 as f64, self.a1 as f64);
        let num = ((b0 + b1 * c).powi(2) + (b1 * s).powi(2)).sqrt();
        let den = ((1.0 + a1 * c).powi(2) + (a1 * s).powi(2)).sqrt();
        num / den
    }
}

/// Clamp a crossover into `(1 Hz, 0.49·fs)` and pre-warp it.
fn prewarp(freq_hz: f32, sample_rate: f32) -> f64 {
    let f = (freq_hz as f64).clamp(1.0, 0.49 * sample_rate as f64);
    (std::f64::consts::PI * f / sample_rate as f64).tan()
}

fn low_shelf(gain: f64, omega: f64) -> FirstOrder {
    let r = gain.sqrt();
    let a0 = 1.0 + omega / r;
    FirstOrder {
        b0: ((1.0 + omega * r) / a0) as f32,
        b1: ((omega * r - 1.0) / a0) as f32,
        a1: ((omega / r - 1.0) / a0) as f32,
        z: 0.0,
    }
}

fn high_shelf(gain: f64, omega: f64) -> FirstOrder {
    let r = gain.sqrt();
    let a0 = 1.0 / r + omega;
    FirstOrder {
        b0: ((r + omega) / a0) as f32,
        b1: ((omega - r) / a0) as f32,
        a1: ((omega - 1.0 / r) / a0) as f32,
        z: 0.0,
    }
}

/// One line's absorption filter: broadband gain × low shelf × high shelf.
/// See the module docs for the design.
#[derive(Clone, Copy, Debug)]
pub struct Absorption {
    gain: f32,
    low: FirstOrder,
    high: FirstOrder,
}

impl Default for Absorption {
    fn default() -> Self {
        Self::lossless()
    }
}

impl Absorption {
    /// Unity at every frequency (a frozen / lossless loop).
    pub const fn lossless() -> Self {
        Self {
            gain: 1.0,
            low: FirstOrder::IDENTITY,
            high: FirstOrder::IDENTITY,
        }
    }

    /// Designed for a line of `delay_samples` against `bands`.
    pub fn new(bands: &DecayBands, delay_samples: f32, sample_rate: f32) -> Self {
        let mut a = Self::lossless();
        a.design(bands, delay_samples, sample_rate);
        a
    }

    /// Redesign for a new target or line length. Keeps the filter state
    /// (no click on a parameter move). No allocation.
    pub fn design(&mut self, bands: &DecayBands, delay_samples: f32, sample_rate: f32) {
        // ln g per band, in f64: a gain within 1e-5 of unity would lose
        // most of its digits in f32.
        let ln = |t60: f32| {
            if t60 == f32::INFINITY {
                return 0.0;
            }
            let t60 = if t60.is_finite() { t60.max(1e-3) } else { 1e-3 } as f64;
            -3.0 * std::f64::consts::LN_10 * delay_samples as f64 / (t60 * sample_rate as f64)
        };
        let (a_l, a_m, a_h) = (ln(bands.t60_low), ln(bands.t60_mid), ln(bands.t60_high));
        let low_hz = bands.low_xover_hz.min(bands.high_xover_hz);
        let high_hz = bands.high_xover_hz.max(bands.low_xover_hz);
        let (om_l, om_h) = (prewarp(low_hz, sample_rate), prewarp(high_hz, sample_rate));
        let mid_hz = (low_hz * high_hz).sqrt();
        let om_m = prewarp(mid_hz, sample_rate);
        let w_m = 2.0 * om_m.atan();
        let alpha = 1.0 / (1.0 + (om_m / om_l).powi(2));
        let r2 = (om_m / om_h).powi(2);
        let beta = r2 / (1.0 + r2);
        let denom = 1.0 - alpha - beta;
        let cap = 0.0f64;
        let shelves = |lg: f64| (low_shelf((a_l - lg).exp(), om_l), high_shelf((a_h - lg).exp(), om_h));
        let mut lg = a_m;
        if denom >= 0.2 {
            lg = ((a_m - alpha * a_l - beta * a_h) / denom).min(cap);
            for _ in 0..3 {
                let (lo, hi) = shelves(lg);
                let mid = lg + lo.magnitude(w_m).ln() + hi.magnitude(w_m).ln();
                lg = (lg + (a_m - mid) / denom).min(cap);
            }
        }
        let (zl, zh) = (self.low.z, self.high.z);
        let (lo, hi) = shelves(lg);
        self.gain = lg.exp() as f32;
        self.low = lo;
        self.high = hi;
        self.low.z = zl;
        self.high.z = zh;
    }

    /// Make the filter lossless (unity, flat), keeping its state.
    pub fn set_lossless(&mut self) {
        let (zl, zh) = (self.low.z, self.high.z);
        *self = Self::lossless();
        self.low.z = zl;
        self.high.z = zh;
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.high.process(self.low.process(x * self.gain))
    }

    /// Magnitude of the designed response at `freq_hz` (analytic, from the
    /// coefficients actually used).
    pub fn magnitude(&self, freq_hz: f32, sample_rate: f32) -> f32 {
        let w = std::f64::consts::TAU * freq_hz as f64 / sample_rate as f64;
        (self.gain as f64 * self.low.magnitude(w) * self.high.magnitude(w)) as f32
    }

    /// Clear the filter state, keeping the design.
    pub fn clear(&mut self) {
        self.low.z = 0.0;
        self.high.z = 0.0;
    }
}
