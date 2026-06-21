//! A tiny deterministic PRNG (SplitMix64) so humanize/groove jitter is
//! reproducible from a seed without pulling in an external crate.

/// SplitMix64 state. Seed it, then draw with [`Rng::next_u64`] /
/// [`Rng::next_bipolar`].
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Seed the generator. Pair `seed` with a per-note salt so notes get
    /// independent-but-deterministic streams:
    /// `Rng::new(seed, note_index)`.
    pub fn new(seed: u64, salt: u64) -> Self {
        Rng {
            state: seed
                .wrapping_add(salt.wrapping_mul(0x9E37_79B9_7F4A_7C15))
                .wrapping_add(0xD1B5_4A32_D192_ED03),
        }
    }

    /// Next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Next value uniformly distributed in `-1.0..=1.0`.
    pub fn next_bipolar(&mut self) -> f64 {
        // 53-bit mantissa → [0,1), then map to [-1, 1).
        let u = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        u * 2.0 - 1.0
    }
}
