//! Stereo decorrelators for widening a mono (or narrow) source.
//!
//! - [`VelvetDecorrelator`]: a sparse ±1 FIR (velvet noise, Alary, Politis
//!   & Välimäki, DAFx-17) filters the mid signal into a **pure side**
//!   component: `L = L + a·D(M)`, `R = R − a·D(M)`. The added component
//!   cancels in `L + R`, so the mono sum is unchanged (up to float
//!   rounding) at any amount. This is the mono-safe widener.
//! - [`AllpassDecorrelator`]: two cascades of ERB-spaced second-order
//!   all-passes, one per side, offset from each other in frequency. Both
//!   channels keep their magnitude response; the mono sum is not
//!   invariant, but the bounded phase difference keeps its ripple small
//!   (measured in `tests/decorrelate.rs`). It smears transients a little.
//!
//! Both are deterministic (fixed seed or fixed layout), fixed-size after
//! construction and allocation-free per sample.

use crate::{Biquad, SimpleRng};

/// Most taps a [`VelvetDecorrelator`] holds.
pub const VELVET_MAX_TAPS: usize = 96;

/// Default velvet filter length in milliseconds.
pub const VELVET_LENGTH_MS: f32 = 30.0;

/// Default impulse density (impulses per second).
pub const VELVET_DENSITY: f32 = 1500.0;

/// Default decay of the tap gains over the filter length, in dB.
pub const VELVET_DECAY_DB: f32 = 20.0;

/// Velvet-noise decorrelator producing pure side from mid.
///
/// One tap per grid segment of `sample_rate / density` samples, at a
/// seeded random position within the segment and a random sign, with an
/// exponentially decaying gain; the gains are normalised to unit energy,
/// so `D` has roughly unity power gain and is uncorrelated with its input
/// for broadband material. For a mono input at amount `a`, the output
/// correlation is `(1 − a²)/(1 + a²)` (0 at amount 1).
///
/// **Band focus:** an optional high-pass and low-pass filter the
/// decorrelated component only, so bass (and, if wanted, the top) stays
/// dry and centred. The filtering happens on the side component, so the
/// mono sum stays invariant whatever the focus.
///
/// Adding a side component raises each channel's level (by
/// `10·log10(1 + a²)` dB on uncorrelated material) while the mono sum
/// stays put; owners that want level-matched stereo compensate outside.
pub struct VelvetDecorrelator {
    buf: Vec<f32>,
    mask: usize,
    write: usize,
    taps: [usize; VELVET_MAX_TAPS],
    gains: [f32; VELVET_MAX_TAPS],
    n_taps: usize,
    amount: f32,
    hp: Biquad,
    hp_on: bool,
    lp: Biquad,
    lp_on: bool,
}

impl VelvetDecorrelator {
    /// A decorrelator with the default length, density and decay.
    /// Allocates its delay buffer (call from `initialize`).
    pub fn new(sample_rate: f32, seed: u64) -> Self {
        Self::with_design(sample_rate, seed, VELVET_LENGTH_MS, VELVET_DENSITY, VELVET_DECAY_DB)
    }

    /// A decorrelator with an explicit design. The tap count is
    /// `length · density`, capped at [`VELVET_MAX_TAPS`].
    pub fn with_design(
        sample_rate: f32,
        seed: u64,
        length_ms: f32,
        density: f32,
        decay_db: f32,
    ) -> Self {
        let sr = sample_rate.max(1.0);
        let len = (length_ms.max(1.0) * 0.001 * sr).ceil() as usize;
        let n = ((length_ms.max(1.0) * 0.001 * density.max(1.0)).round() as usize)
            .clamp(1, VELVET_MAX_TAPS);
        let seg = len as f32 / n as f32;
        let mut rng = SimpleRng::new(splitmix64(seed));
        let mut taps = [0usize; VELVET_MAX_TAPS];
        let mut gains = [0.0f32; VELVET_MAX_TAPS];
        let mut energy = 0.0f64;
        for k in 0..n {
            let r = unit(&mut rng);
            let pos = (k as f32 * seg + r * (seg - 1.0).max(0.0)).round() as usize;
            // Never tap delay 0: an undelayed tap is correlated with the
            // input and only shifts the balance.
            taps[k] = pos.max(1);
            let sign = if rng.next_u32() & 1 == 0 { 1.0 } else { -1.0 };
            let t = k as f32 / n as f32;
            let g = sign * crate::db_to_linear(-decay_db.max(0.0) * t);
            gains[k] = g;
            energy += (g as f64) * (g as f64);
        }
        let norm = (1.0 / energy.max(1e-12)).sqrt() as f32;
        for g in &mut gains[..n] {
            *g *= norm;
        }
        let size = (taps[..n].iter().copied().max().unwrap_or(1) + 1).next_power_of_two();
        Self {
            buf: vec![0.0; size],
            mask: size - 1,
            write: 0,
            taps,
            gains,
            n_taps: n,
            amount: 0.0,
            hp: Biquad::identity(),
            hp_on: false,
            lp: Biquad::identity(),
            lp_on: false,
        }
    }

    /// Number of taps in the velvet filter.
    pub fn tap_count(&self) -> usize {
        self.n_taps
    }

    /// Side amount `a`, clamped to 0..=1. 0 bypasses (exact passthrough).
    pub fn set_amount(&mut self, amount: f32) {
        self.amount = if amount > 0.0 { amount.min(1.0) } else { 0.0 };
    }

    /// Restrict the decorrelated component to `low_hz..high_hz`
    /// (second-order Butterworth each). `low_hz` ≤ 0 or `high_hz` ≤ 0
    /// turns that side of the band off.
    pub fn set_focus(&mut self, sample_rate: f32, low_hz: f32, high_hz: f32) {
        let q = std::f32::consts::FRAC_1_SQRT_2;
        self.hp_on = low_hz > 0.0;
        if self.hp_on {
            self.hp.set_high_pass(sample_rate, low_hz, q);
        }
        self.lp_on = high_hz > 0.0;
        if self.lp_on {
            self.lp.set_low_pass(sample_rate, high_hz, q);
        }
    }

    pub fn reset(&mut self) {
        self.buf.fill(0.0);
        self.write = 0;
        self.hp.reset();
        self.lp.reset();
    }

    /// Push one mid sample and return the (focused, scaled) side
    /// component `a·D(M)`.
    #[inline]
    pub fn side(&mut self, mid: f32) -> f32 {
        let mid = if mid.is_finite() { mid } else { 0.0 };
        self.buf[self.write] = mid;
        let w = self.write;
        self.write = (self.write + 1) & self.mask;
        if self.amount == 0.0 {
            return 0.0;
        }
        let mut acc = 0.0f32;
        for k in 0..self.n_taps {
            acc += self.gains[k] * self.buf[w.wrapping_sub(self.taps[k]) & self.mask];
        }
        if self.hp_on {
            acc = self.hp.process(acc);
        }
        if self.lp_on {
            acc = self.lp.process(acc);
        }
        acc * self.amount
    }

    /// Widen one stereo frame: `(L + s, R − s)` with `s = side((L+R)/2)`.
    /// At amount 0 it returns the input unchanged.
    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let s = self.side(0.5 * (l + r));
        if self.amount == 0.0 {
            return (l, r);
        }
        (l + s, r - s)
    }
}

/// Spread nearby seeds apart (`SimpleRng` alone maps 42 and 43 to the
/// same state).
fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A uniform value in [0, 1).
fn unit(rng: &mut SimpleRng) -> f32 {
    (rng.next_u32() >> 8) as f32 / (1u32 << 24) as f32
}

/// Most sections per side of an [`AllpassDecorrelator`].
pub const ALLPASS_MAX_SECTIONS: usize = 50;

/// ERB-number (Glasberg & Moore) of a frequency in Hz.
pub fn erb_number(freq_hz: f32) -> f32 {
    21.4 * (1.0 + 0.00437 * freq_hz).log10()
}

/// Frequency in Hz of an ERB-number.
pub fn erb_number_to_hz(erb: f32) -> f32 {
    (10f32.powf(erb / 21.4) - 1.0) / 0.00437
}

/// All-pass decorrelation cascade: per side, `sections` second-order
/// all-passes at ERB-spaced centres between `low_hz` and `high_hz`. The
/// right side's centres sit `spread` of one ERB step above the left's, so
/// the two channels drift apart in phase by a bounded amount (about
/// `2π·spread` in the covered band, falling to 0 below `low_hz`, which
/// keeps the bass in phase). `spread` 0 makes both sides identical.
///
/// Each channel's magnitude response is flat (all-pass); `L + R` of a
/// mono input dips where the phase difference peaks. Coefficients are
/// set by [`AllpassDecorrelator::configure`] (block-rate); `process` is
/// per sample. Starts bypassed (0 sections).
#[derive(Clone, Copy, Debug)]
pub struct AllpassDecorrelator {
    left: [Biquad; ALLPASS_MAX_SECTIONS],
    right: [Biquad; ALLPASS_MAX_SECTIONS],
    n: usize,
}

impl Default for AllpassDecorrelator {
    fn default() -> Self {
        Self {
            left: [Biquad::identity(); ALLPASS_MAX_SECTIONS],
            right: [Biquad::identity(); ALLPASS_MAX_SECTIONS],
            n: 0,
        }
    }
}

impl AllpassDecorrelator {
    /// Default section count per side.
    pub const DEFAULT_SECTIONS: usize = 40;
    /// Default band.
    pub const DEFAULT_LOW_HZ: f32 = 150.0;
    pub const DEFAULT_HIGH_HZ: f32 = 16_000.0;
    /// Default right-side offset, as a fraction of the ERB step.
    pub const DEFAULT_SPREAD: f32 = 0.2;
    /// Section Q. Low enough that neighbouring sections overlap and the
    /// cascade's phase is a smooth ramp rather than a staircase.
    pub const SECTION_Q: f32 = 0.35;

    /// A cascade with the default layout.
    pub fn new(sample_rate: f32) -> Self {
        let mut s = Self::default();
        s.configure(
            sample_rate,
            Self::DEFAULT_SECTIONS,
            Self::DEFAULT_LOW_HZ,
            Self::DEFAULT_HIGH_HZ,
            Self::DEFAULT_SPREAD,
        );
        s
    }

    /// Lay out `sections` (≤ [`ALLPASS_MAX_SECTIONS`]; 0 bypasses) per
    /// side between `low_hz` and `high_hz`, with the right side offset by
    /// `spread` (0..=0.5) of an ERB step. Keeps the filter state.
    pub fn configure(
        &mut self,
        sample_rate: f32,
        sections: usize,
        low_hz: f32,
        high_hz: f32,
        spread: f32,
    ) {
        self.n = sections.min(ALLPASS_MAX_SECTIONS);
        if self.n == 0 {
            return;
        }
        let spread = spread.clamp(0.0, 0.5);
        let e_lo = erb_number(low_hz.max(10.0));
        let e_hi = erb_number(high_hz.max(low_hz.max(10.0) + 1.0));
        let step = if self.n > 1 {
            (e_hi - e_lo) / (self.n - 1) as f32
        } else {
            0.0
        };
        for k in 0..self.n {
            let e = e_lo + k as f32 * step;
            self.left[k].set_all_pass(sample_rate, erb_number_to_hz(e), Self::SECTION_Q);
            let er = e + spread * step;
            self.right[k].set_all_pass(sample_rate, erb_number_to_hz(er), Self::SECTION_Q);
        }
    }

    /// Sections per side currently in use.
    pub fn sections(&self) -> usize {
        self.n
    }

    pub fn reset(&mut self) {
        for b in self.left.iter_mut().chain(self.right.iter_mut()) {
            b.reset();
        }
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let (mut l, mut r) = (l, r);
        for k in 0..self.n {
            l = self.left[k].process(l);
            r = self.right[k].process(r);
        }
        (l, r)
    }
}
