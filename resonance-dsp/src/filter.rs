/// One-pole lowpass filter for frequency-dependent damping.
pub struct OnePole {
    state: f32,
    coeff: f32,
}

impl OnePole {
    pub fn new() -> Self {
        Self {
            state: 0.0,
            coeff: 0.0,
        }
    }

    /// Set cutoff frequency (Hz) for given sample rate.
    pub fn set_cutoff(&mut self, freq_hz: f32, sample_rate: f32) {
        let w = (2.0 * std::f32::consts::PI * freq_hz / sample_rate).min(std::f32::consts::PI);
        self.coeff = (-w).exp();
    }

    pub fn process(&mut self, input: f32) -> f32 {
        self.state = input + self.coeff * (self.state - input);
        self.state
    }

    pub fn clear(&mut self) {
        self.state = 0.0;
    }
}

impl Default for OnePole {
    fn default() -> Self {
        Self::new()
    }
}
