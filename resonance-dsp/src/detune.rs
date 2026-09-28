//! Short-delay Doppler detune: a delay-line pitch shifter for small
//! shifts (the micro-shift / "H910" widening trick).
//!
//! [`DopplerShifter`] reads a delay line through two taps whose delay
//! ramps linearly, half a cycle apart. A delay that changes at
//! `d'(t) = 1 − ratio` samples per sample plays the input back at
//! `ratio` times its frequency (the Doppler effect of a moving read
//! head); when a tap's ramp runs out of window it jumps back, and a
//! `sin²` crossfade hides the jump behind the other tap. The two gains
//! sum to exactly 1 at every phase, so a steady input keeps its level.
//!
//! Meant for shifts of a few to a few tens of cents, where the window
//! can be long (tens of ms) and the crossfade cycle slow (seconds), so
//! the crossfade's amplitude modulation is inaudible. It is not a
//! general transposer. Deterministic, allocation-free per sample, and
//! fixed-size after construction.
//!
//! The output is always delayed: by `base + window·phase` per tap,
//! `base + window/2` on average ([`DopplerShifter::mean_delay_samples`]).
//! At 0 cents the phase never moves and the output is exactly the input
//! delayed by that mean.

/// Delay-line pitch shifter for small shifts. See the module docs.
pub struct DopplerShifter {
    buf: Vec<f32>,
    mask: usize,
    write: usize,
    /// Tap 1's position in its ramp, 0..1. Tap 2 sits half a cycle on.
    /// `f64`: at ±10 cents the per-sample step is ~5e-6, which `f32`
    /// would round by whole percent of the step near 1.0.
    phase: f64,
    /// Phase increment per sample: `(1 − ratio) / window`.
    step: f64,
    /// Ramp length in samples.
    window: f32,
    /// Fixed delay under the ramp, in samples.
    base: f32,
    /// Largest `base` the buffer holds, in samples.
    max_base: f32,
}

impl DopplerShifter {
    /// A shifter whose base delay may reach `max_base_ms` and whose ramp
    /// is `window_ms` long, at 0 cents with base 0. Allocates its delay
    /// line (call from `initialize`).
    pub fn new(sample_rate: f32, max_base_ms: f32, window_ms: f32) -> Self {
        let sr = sample_rate.max(1.0);
        // Whole samples, so a 0-cent shifter is an exact integer delay.
        let window = ms_to_samples(window_ms.max(0.1), sr).round().max(2.0);
        let max_base = ms_to_samples(max_base_ms.max(0.0), sr).ceil();
        let size = ((max_base + window).ceil() as usize + 4).next_power_of_two();
        Self {
            buf: vec![0.0; size],
            mask: size - 1,
            write: 0,
            phase: 0.0,
            step: 0.0,
            window,
            base: 0.0,
            max_base,
        }
    }

    /// Set the shift in cents (positive = up). Block-rate.
    pub fn set_cents(&mut self, cents: f32) {
        let cents = if cents.is_finite() { cents.clamp(-1200.0, 1200.0) } else { 0.0 };
        let ratio = 2f64.powf(cents as f64 / 1200.0);
        self.step = (1.0 - ratio) / self.window as f64;
    }

    /// Set the fixed delay under the ramp, clamped to the constructed
    /// maximum. Block-rate.
    pub fn set_base_delay(&mut self, sample_rate: f32, base_ms: f32) {
        let base = if base_ms.is_finite() { ms_to_samples(base_ms.max(0.0), sample_rate) } else { 0.0 };
        self.base = base.min(self.max_base);
    }

    /// Average delay of the output relative to the input, in samples.
    pub fn mean_delay_samples(&self) -> f32 {
        self.base + 0.5 * self.window
    }

    /// Ramp length in samples.
    pub fn window_samples(&self) -> f32 {
        self.window
    }

    pub fn reset(&mut self) {
        self.buf.fill(0.0);
        self.write = 0;
        self.phase = 0.0;
    }

    /// Read `delay` samples back from the newest sample (0 = newest).
    /// 4-point Hermite between the two neighbouring samples; a linear
    /// read would take up to 3 dB off broadband material at half-sample
    /// delays. Inside the newest sample (no newer neighbour) it falls
    /// back to linear.
    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let i = delay as usize;
        let frac = delay - i as f32;
        let at = |k: usize| self.buf[self.write.wrapping_sub(k) & self.mask];
        if frac == 0.0 {
            return at(i);
        }
        if i == 0 {
            let (a, b) = (at(0), at(1));
            return a + frac * (b - a);
        }
        // Forward in time: older `x0` at `i + 1`, newer `x1` at `i`.
        crate::hermite4(at(i + 2), at(i + 1), at(i), at(i - 1), 1.0 - frac)
    }

    /// Push one sample and return the shifted, delayed output.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.buf[self.write] = if x.is_finite() { x } else { 0.0 };
        let p1 = self.phase;
        let mut p2 = p1 + 0.5;
        if p2 >= 1.0 {
            p2 -= 1.0;
        }
        let g1 = (std::f64::consts::PI * p1).sin().powi(2) as f32;
        let g2 = (std::f64::consts::PI * p2).sin().powi(2) as f32;
        let d1 = self.base + self.window * p1 as f32;
        let d2 = self.base + self.window * p2 as f32;
        let mut y = 0.0;
        if g1 != 0.0 {
            y += g1 * self.read(d1);
        }
        if g2 != 0.0 {
            y += g2 * self.read(d2);
        }
        self.write = (self.write + 1) & self.mask;
        self.phase += self.step;
        self.phase -= self.phase.floor();
        y
    }
}

/// `ms` at `sr` in samples, computed in `f64` so whole-sample delays
/// (10 ms at 48 kHz) come out whole.
fn ms_to_samples(ms: f32, sr: f32) -> f32 {
    (ms as f64 * sr as f64 / 1000.0) as f32
}
