/// Simple delay line with power-of-2 buffer for cheap wrapping.
pub struct DelayLine {
    buffer: Vec<f32>,
    mask: usize,
    write_pos: usize,
}

impl DelayLine {
    pub fn new(max_samples: usize) -> Self {
        let size = max_samples.max(2).next_power_of_two();
        Self {
            buffer: vec![0.0; size],
            mask: size - 1,
            write_pos: 0,
        }
    }

    pub fn push(&mut self, sample: f32) {
        self.buffer[self.write_pos] = sample;
        self.write_pos = (self.write_pos + 1) & self.mask;
    }

    pub fn tap(&self, delay: usize) -> f32 {
        let idx = self.write_pos.wrapping_sub(delay).wrapping_sub(1) & self.mask;
        self.buffer[idx]
    }

    /// Read with linear interpolation for fractional delays (modulation).
    pub fn tap_linear(&self, delay_frac: f32) -> f32 {
        let delay_int = delay_frac as usize;
        let frac = delay_frac - delay_int as f32;
        let a = self.tap(delay_int);
        let b = self.tap(delay_int + 1);
        a + frac * (b - a)
    }

    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }
}
