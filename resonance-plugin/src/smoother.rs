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

impl Smoother {
    pub fn new(style: SmoothingStyle) -> Self {
        Self {
            style,
            sample_rate: 44100.0,
            current: 0.0,
            target: 0.0,
            step: 0.0,
            remaining: 0,
            ramp_samples: 0,
        }
    }

    /// Update the sample rate and recompute ramp length.
    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
        self.ramp_samples = match self.style {
            SmoothingStyle::None => 0,
            SmoothingStyle::Linear(ms) | SmoothingStyle::Logarithmic(ms) => {
                (sr * ms / 1000.0).ceil() as u32
            }
        };
    }

    /// Set a new target value and begin smoothing toward it.
    pub fn set_target(&mut self, target: f32) {
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
                    // Exponential decay coefficient: reaches ~95% in ramp_samples
                    // coeff = 1 - e^(-3 / ramp_samples) gives ~95% convergence
                    self.step = 1.0 - (-3.0 / self.ramp_samples as f32).exp();
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

    /// Whether the smoother is currently ramping.
    pub fn is_smoothing(&self) -> bool {
        self.remaining > 0
    }
}
