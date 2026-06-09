//! Small deterministic PRNG used by the generators in this crate.
//!
//! We ship our own xorshift64 rather than pulling in `rand`/`fastrand` so
//! this crate stays dep-free apart from `serde`. The app crate's drumroll
//! humanizer uses the same algorithm — picking different impls here would
//! only confuse readers.
//!
//! This is the crate's *single* determinism contract: every seeded
//! generator (progressions, motifs, vocal styles, Markov sampling)
//! draws from `XorShift`, so a given seed reproduces the same output
//! forever — unlike `rand`'s `SmallRng`, whose stream may change
//! between `rand` versions. Do not reintroduce a second RNG.

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
    ///
    /// Uses rejection sampling to kill the modulo bias: a raw draw
    /// landing in the final partial bucket (the top `2^64 mod n`
    /// values of the `u64` range) is redrawn, so every residue is
    /// equally likely. The accepted path keeps the historical `x % n`
    /// mapping, and for the small `n` used in this crate the rejection
    /// probability is ~`n / 2^64` per draw — so in practice the
    /// deterministic sequences produced before this change are
    /// preserved. (The residual 1-in-2^64 skew from xorshift64 never
    /// emitting 0 is inherent to the generator and ignored.)
    pub(crate) fn next_range(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        let n64 = n as u64;
        // 2^64 mod n: size of the final partial bucket.
        let rem = (u64::MAX % n64 + 1) % n64;
        loop {
            let x = self.next_u64();
            if rem == 0 || x <= u64::MAX - rem {
                return (x % n64) as usize;
            }
        }
    }
}
