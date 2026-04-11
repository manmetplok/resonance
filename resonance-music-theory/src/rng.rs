//! Small deterministic PRNG used by the generators in this crate.
//!
//! We ship our own xorshift64 rather than pulling in `rand`/`fastrand` so
//! this crate stays dep-free apart from `serde`. The app crate's drumroll
//! humanizer uses the same algorithm — picking different impls here would
//! only confuse readers.

pub(crate) struct XorShift {
    state: u64,
}

impl XorShift {
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x9E3779B97F4A7C15 } else { seed },
        }
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Uniform in [0, 1).
    pub(crate) fn next_f32(&mut self) -> f32 {
        let bits = (self.next_u64() >> 40) as u32;
        (bits as f32) / ((1u32 << 24) as f32)
    }

    /// Uniform integer in `[0, n)`. Returns 0 if `n == 0`.
    pub(crate) fn next_range(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() as usize) % n
        }
    }
}
