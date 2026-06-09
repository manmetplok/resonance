/// Minimal deterministic PRNG (xorshift32) for delay time randomization.
pub struct SimpleRng {
    state: u32,
}

impl SimpleRng {
    pub fn new(seed: u64) -> Self {
        // `| 1` forces the state odd, so even a seed of 0 (or one whose
        // halves XOR to 0) cannot produce the all-zero state xorshift
        // would be stuck at forever.
        Self {
            state: ((seed ^ (seed >> 32)) as u32) | 1, // ensure non-zero, mix both halves
        }
    }

    pub fn next_u32(&mut self) -> u32 {
        // A zero state is a fixed point of xorshift: every subsequent
        // output would be 0. `new` makes it unreachable via `| 1`; this
        // assert catches any future code path that writes `state`
        // directly. Release builds stay branch-free.
        debug_assert!(self.state != 0, "SimpleRng: zero state is a fixed point");
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state
    }
}
