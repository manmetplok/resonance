//! Tiny single-channel fixed-length delay line used to align the raw
//! multiband input with the output of the linear-phase lowpass filters.

pub struct DelayLine {
    buffer: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    pub fn new(len: usize) -> Self {
        Self {
            buffer: vec![0.0; len.max(1)],
            pos: 0,
        }
    }

    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.pos = 0;
    }

    /// Write `input` to the newest slot and return the oldest slot.
    #[inline]
    pub fn push(&mut self, input: f32) -> f32 {
        let out = self.buffer[self.pos];
        self.buffer[self.pos] = input;
        self.pos += 1;
        if self.pos == self.buffer.len() {
            self.pos = 0;
        }
        out
    }
}
