/// Minimal deterministic PRNG (xorshift32) for delay time randomization.
pub struct SimpleRng {
    state: u32,
}

impl SimpleRng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: (seed as u32) | 1, // ensure non-zero
        }
    }

    pub fn next_u32(&mut self) -> u32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state
    }
}
