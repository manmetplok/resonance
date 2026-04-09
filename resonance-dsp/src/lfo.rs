/// Simple sine LFO.
pub struct Lfo {
    phase: f32,
    phase_inc: f32,
}

impl Lfo {
    pub fn new(rate_hz: f32, sample_rate: f32, initial_phase: f32) -> Self {
        Self {
            phase: initial_phase,
            phase_inc: rate_hz / sample_rate,
        }
    }

    pub fn set_rate(&mut self, rate_hz: f32, sample_rate: f32) {
        self.phase_inc = rate_hz / sample_rate;
    }

    pub fn next(&mut self) -> f32 {
        let out = (self.phase * 2.0 * std::f32::consts::PI).sin();
        self.phase += self.phase_inc;
        self.phase -= self.phase.floor();
        out
    }
}
