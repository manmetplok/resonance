//! Schroeder allpass with an optional modulated fractional length and an
//! optional inner processor in its delay path (nesting), the building
//! block of Dattorro's plate (reverb-algorithms.md §4.4).
//!
//! ```text
//!   w[n] = x[n] + g · s[n]          s[n] = inner(w[n − M])
//!   y[n] = s[n] − g · w[n]
//! ```
//!
//! With an identity inner this is `H(z) = (−g + z^−M) / (1 − g·z^−M)`,
//! flat in magnitude for any `|g| < 1`. Any allpass inner (another
//! `Allpass`, a plain delay) keeps the whole structure allpass, which is
//! how nested allpasses are built: pass a closure that runs the inner
//! stage to [`Allpass::process_nested`].
//!
//! Dattorro writes the decay diffusers with a negative coefficient: pass
//! `g` with the sign the paper uses.
//!
//! A fractional (modulated) length is read through a first-order
//! **allpass interpolator** ([`allpass_read`]), not a linear one: linear
//! interpolation is a low-pass (−3 dB at 12 kHz at a half-sample fraction)
//! and inside a feedback loop it would shorten the treble decay on every
//! pass. The allpass interpolator is flat in magnitude; its only cost is a
//! small transient when the fraction moves, inaudible at the slow rates
//! reverbs modulate at. An unmodulated read is an integer tap.

use crate::DelayLine;

pub struct Allpass {
    line: DelayLine,
    interp: f32,
    delay: usize,
    max_delay: usize,
    g: f32,
}

impl Allpass {
    /// An allpass of `delay` samples (≥ 1) with coefficient `g`. The line
    /// is sized for lengths up to `max_delay` (plus modulation depth the
    /// caller adds on top, which must stay within it).
    pub fn new(max_delay: usize, delay: usize, g: f32) -> Self {
        let max_delay = max_delay.max(delay).max(2);
        Self {
            line: DelayLine::new(max_delay + 2),
            interp: 0.0,
            delay: delay.clamp(1, max_delay),
            max_delay,
            g,
        }
    }

    pub fn set_delay(&mut self, delay: usize) {
        self.delay = delay.clamp(1, self.max_delay);
    }

    pub fn delay(&self) -> usize {
        self.delay
    }

    pub fn set_gain(&mut self, g: f32) {
        self.g = g;
    }

    pub fn gain(&self) -> f32 {
        self.g
    }

    /// One sample at the nominal integer length.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let s = self.line.tap(self.delay - 1);
        self.finish(x, s)
    }

    /// One sample with the length moved by `offset` samples (fractional,
    /// either sign; the effective length is clamped to `[1, max_delay]`).
    #[inline]
    pub fn process_modulated(&mut self, x: f32, offset: f32) -> f32 {
        let s = self.read(offset);
        self.finish(x, s)
    }

    /// One sample with `inner` applied to the delayed signal before it is
    /// fed back and forward (a nested allpass when `inner` is allpass).
    #[inline]
    pub fn process_nested<F: FnOnce(f32) -> f32>(&mut self, x: f32, offset: f32, inner: F) -> f32 {
        let s = inner(self.read(offset));
        self.finish(x, s)
    }

    /// Read the internal line `delay` samples back (`0` = the value written
    /// on the last call). Dattorro's output taps read inside his allpasses.
    #[inline]
    pub fn tap(&self, delay: usize) -> f32 {
        self.line.tap(delay)
    }

    pub fn clear(&mut self) {
        self.line.clear();
        self.interp = 0.0;
    }

    #[inline]
    fn read(&mut self, offset: f32) -> f32 {
        if offset == 0.0 {
            let s = self.line.tap(self.delay - 1);
            self.interp = s;
            return s;
        }
        let d = (self.delay as f32 + offset).clamp(1.0, self.max_delay as f32);
        allpass_read(&self.line, d - 1.0, &mut self.interp)
    }

    #[inline]
    fn finish(&mut self, x: f32, s: f32) -> f32 {
        let w = x + self.g * s;
        self.line.push(w);
        s - self.g * w
    }
}

/// Read `line` at the fractional position `pos` (in [`DelayLine::tap`]
/// units, `pos ≥ 0.5`) through a first-order allpass interpolator
/// `H(z) = (η + z⁻¹)/(1 + η·z⁻¹)`, `η = (1 − Δ)/(1 + Δ)`.
///
/// The integer part is chosen so the fractional delay `Δ` stays in
/// `[0.5, 1.5)`, which keeps `|η| ≤ 1/3`: the pole stays well inside the
/// unit circle and the transient when `Δ` moves dies in a few samples.
/// `state` is the interpolator's previous output, one per read head; an
/// integer read should still store its value there so switching between
/// integer and fractional reads is continuous. At an integer `pos`
/// (`Δ = 1`, `η = 0`) this is exactly `line.tap(pos)`.
#[inline]
pub fn allpass_read(line: &DelayLine, pos: f32, state: &mut f32) -> f32 {
    let i = (pos - 0.5).floor().max(0.0);
    let delta = pos - i;
    let eta = (1.0 - delta) / (1.0 + delta);
    let iu = i as usize;
    let y = eta * (line.tap(iu) - *state) + line.tap(iu + 1);
    *state = y;
    y
}
