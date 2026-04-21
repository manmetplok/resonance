/// Multi-shape LFO: sine, triangle, saw, square, sample & hold.
use resonance_dsp::SimpleRng;

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum LfoShape {
    Sine = 0,
    Triangle = 1,
    Saw = 2,
    Square = 3,
    SampleAndHold = 4,
}

impl LfoShape {
    pub fn from_int(v: i32) -> Self {
        match v {
            0 => Self::Sine,
            1 => Self::Triangle,
            2 => Self::Saw,
            3 => Self::Square,
            4 => Self::SampleAndHold,
            _ => Self::Sine,
        }
    }
}

#[derive(Clone)]
pub struct MultiLfo {
    pub phase: f32,
    phase_inc: f32,
    prev_phase: f32,
    sh_value: f32,
}

impl MultiLfo {
    pub fn new() -> Self {
        Self {
            phase: 0.0,
            phase_inc: 0.0,
            prev_phase: 0.0,
            sh_value: 0.0,
        }
    }

    pub fn set_rate(&mut self, rate_hz: f32, sample_rate: f32) {
        self.phase_inc = rate_hz / sample_rate;
    }

    pub fn reset_phase(&mut self) {
        self.phase = 0.0;
        self.prev_phase = 0.0;
    }

    /// Advance one sample. Returns value in -1..1.
    pub fn next(&mut self, shape: LfoShape, rng: &mut SimpleRng) -> f32 {
        let out = match shape {
            LfoShape::Sine => (self.phase * std::f32::consts::TAU).sin(),
            LfoShape::Triangle => {
                if self.phase < 0.25 {
                    self.phase * 4.0
                } else if self.phase < 0.75 {
                    2.0 - self.phase * 4.0
                } else {
                    self.phase * 4.0 - 4.0
                }
            }
            LfoShape::Saw => 2.0 * self.phase - 1.0,
            LfoShape::Square => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoShape::SampleAndHold => {
                // Latch new random value on phase wrap
                if self.phase < self.prev_phase {
                    self.sh_value = (rng.next_u32() as f32 / u32::MAX as f32) * 2.0 - 1.0;
                }
                self.sh_value
            }
        };

        self.prev_phase = self.phase;
        self.phase += self.phase_inc;
        self.phase -= self.phase.floor();

        out
    }
}
