//! Per-sample parameter smoothing.

/// The smoothing algorithm to use.
#[derive(Clone, Copy)]
pub enum SmoothingStyle {
    /// No smoothing -- value changes instantly.
    None,
    /// Linear ramp over the given duration in milliseconds.
    Linear(f32),
    /// Logarithmic (exponential) ramp over the given duration in milliseconds.
    Logarithmic(f32),
}

/// A per-sample smoother for parameter values.
pub struct Smoother {
    style: SmoothingStyle,
    sample_rate: f32,
    current: f32,
    target: f32,
    /// Per-sample step for linear smoothing, or coefficient for logarithmic.
    step: f32,
    /// Samples remaining in the current ramp.
    remaining: u32,
    /// Total ramp length in samples (cached from style + sample_rate).
    ramp_samples: u32,
}

/// Sample rate a smoother assumes until its owner calls
/// [`Smoother::set_sample_rate`].
const DEFAULT_SAMPLE_RATE: f32 = 44_100.0;

/// How close a logarithmic ramp gets to its target, as a fraction of the
/// distance it started from, before the final sample lands on it: the
/// snap at the end of the ramp is at most this big (−80 dB of the step).
/// The old `1 − e^(−3/N)` coefficient left ~5 % of the distance for that
/// snap — a −26 dB step on a 0 → 1 gain change (code review HOST-09).
const LOG_RAMP_RESIDUAL: f32 = 1e-4;

impl Smoother {
    /// A smoother for `style`, already configured for
    /// [`DEFAULT_SAMPLE_RATE`]: a plugin that never calls
    /// [`Self::set_sample_rate`] still smooths (at a slightly wrong
    /// length if it runs at another rate) rather than jumping on every
    /// change (HOST-09). Owners should still call `set_sample_rate` from
    /// `initialize`.
    pub fn new(style: SmoothingStyle) -> Self {
        let mut s = Self {
            style,
            sample_rate: DEFAULT_SAMPLE_RATE,
            current: 0.0,
            target: 0.0,
            step: 0.0,
            remaining: 0,
            ramp_samples: 0,
        };
        s.set_sample_rate(DEFAULT_SAMPLE_RATE);
        s
    }

    /// Update the sample rate and recompute ramp length.
    pub fn set_sample_rate(&mut self, sr: f32) {
        debug_assert!(
            sr.is_finite() && sr > 0.0,
            "Smoother::set_sample_rate({sr}): not a sample rate"
        );
        self.sample_rate = sr;
        self.ramp_samples = match self.style {
            SmoothingStyle::None => 0,
            SmoothingStyle::Linear(ms) | SmoothingStyle::Logarithmic(ms) => {
                (sr * ms / 1000.0).ceil() as u32
            }
        };
    }

    /// Set a new target value and begin smoothing toward it.
    ///
    /// A target equal to the current one is a no-op: a ramp in flight
    /// keeps its step and lands on time. Owners call this once per block
    /// with the param's value, and restarting the ramp every block would
    /// make it approach the target geometrically and never land on it —
    /// which costs every exact-at-target fast path (`mix == 1`, gain 1)
    /// its bit-exactness.
    pub fn set_target(&mut self, target: f32) {
        if target == self.target {
            return;
        }
        self.target = target;
        match self.style {
            SmoothingStyle::None => {
                self.current = target;
                self.remaining = 0;
            }
            SmoothingStyle::Linear(_) => {
                if self.ramp_samples == 0 {
                    self.current = target;
                    self.remaining = 0;
                } else {
                    self.step = (target - self.current) / self.ramp_samples as f32;
                    self.remaining = self.ramp_samples;
                }
            }
            SmoothingStyle::Logarithmic(_) => {
                if self.ramp_samples == 0 {
                    self.current = target;
                    self.remaining = 0;
                } else {
                    // Exponential decay coefficient chosen so that
                    // (1 - coeff)^ramp_samples == LOG_RAMP_RESIDUAL: after
                    // the nominal ramp only that fraction of the distance
                    // is left for the final snap onto the target.
                    self.step =
                        1.0 - (LOG_RAMP_RESIDUAL.ln() / self.ramp_samples as f32).exp();
                    self.remaining = self.ramp_samples;
                }
            }
        }
    }

    /// Reset the smoother to a specific value without ramping.
    pub fn reset(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.remaining = 0;
        self.step = 0.0;
    }

    /// Analytically fast-forward the smoother by `n` samples. Produces the
    /// same `current` value as `for _ in 0..n { self.next(); }` but without
    /// the per-sample loop. Used for block-rate parameters where only the
    /// end-of-block value is consumed (e.g. expensive DSP coefficient
    /// updates), and where spinning `frames` iterations purely to advance
    /// smoother state would be wasteful.
    pub fn skip(&mut self, n: u32) {
        if n == 0 {
            return;
        }
        if self.remaining == 0 {
            self.current = self.target;
            return;
        }

        let consumed = n.min(self.remaining);
        let reaches_target = consumed == self.remaining;
        self.remaining -= consumed;

        match self.style {
            SmoothingStyle::None => {
                self.current = self.target;
            }
            SmoothingStyle::Linear(_) => {
                if reaches_target {
                    self.current = self.target;
                } else {
                    self.current += self.step * consumed as f32;
                }
            }
            SmoothingStyle::Logarithmic(_) => {
                if reaches_target {
                    self.current = self.target;
                } else {
                    // Exact closed form for the recurrence
                    //   c_{k+1} = c_k + (t - c_k) * step
                    // which gives
                    //   c_n = t - (t - c_0) * (1 - step)^n
                    let remaining_dist = self.target - self.current;
                    let decay = (1.0 - self.step).powi(consumed as i32);
                    self.current = self.target - remaining_dist * decay;
                }
            }
        }
    }

    /// Get the next smoothed value (call once per sample).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f32 {
        if self.remaining == 0 {
            self.current = self.target;
            return self.current;
        }

        self.remaining -= 1;

        match self.style {
            SmoothingStyle::None => {
                self.current = self.target;
            }
            SmoothingStyle::Linear(_) => {
                self.current += self.step;
                if self.remaining == 0 {
                    self.current = self.target;
                }
            }
            SmoothingStyle::Logarithmic(_) => {
                self.current += (self.target - self.current) * self.step;
                if self.remaining == 0 {
                    self.current = self.target;
                }
            }
        }

        self.current
    }

    /// Get the current value without advancing.
    pub fn current(&self) -> f32 {
        self.current
    }

}
