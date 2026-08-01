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

    /// Value at the current phase, in -1..1, **without** advancing.
    ///
    /// Split out from [`Self::advance`] because the two run at different
    /// rates on the audio path: the phase has to move every sample to stay
    /// continuous, but the value is only consumed by the modulation matrix,
    /// which is evaluated at control rate. For the default sine shape this
    /// is a `sin()` call — at 32 voices × 3 LFOs × 48 kHz, keeping it off
    /// the per-sample path is worth several million transcendental calls a
    /// second.
    #[inline]
    pub fn value(&self, shape: LfoShape) -> f32 {
        match shape {
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
            LfoShape::SampleAndHold => self.sh_value,
        }
    }

    /// Advance the phase by one sample.
    ///
    /// Must be called once per sample regardless of whether [`Self::value`]
    /// was read, so LFO phase stays sample-accurate and the S&H latch fires
    /// on the exact wrap sample.
    #[inline]
    pub fn advance(&mut self, shape: LfoShape, rng: &mut SimpleRng) {
        self.prev_phase = self.phase;
        self.phase += self.phase_inc;
        self.phase -= self.phase.floor();

        // Latch a new S&H value when the phase wrapped on this advance.
        // Done after the value read so the value held for *this* sample
        // matches what the user saw the previous frame, and the new
        // random value is what subsequent samples in this cycle hear.
        // Pulling the RNG out of the pre-advance match avoids calling
        // it for every other LFO shape (the old code ran the RNG
        // unconditionally inside the SH branch even when the phase
        // hadn't wrapped — multiplied across 32 voices × 3 LFOs that
        // was a few million unused RNG calls per second).
        if matches!(shape, LfoShape::SampleAndHold) && self.phase < self.prev_phase {
            self.sh_value = (rng.next_u32() as f32 / u32::MAX as f32) * 2.0 - 1.0;
        }
    }

    /// Read the current value, then advance one sample.
    ///
    /// Exactly `value()` followed by `advance()`; kept for callers that need
    /// both every sample.
    #[inline]
    pub fn next(&mut self, shape: LfoShape, rng: &mut SimpleRng) -> f32 {
        let out = self.value(shape);
        self.advance(shape, rng);
        out
    }
}

impl Default for MultiLfo {
    fn default() -> Self {
        Self::new()
    }
}
