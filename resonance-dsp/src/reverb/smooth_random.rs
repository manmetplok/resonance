//! Smoothed random modulator: the reverb engines' replacement for sine
//! LFOs on delay taps (reverb-algorithms.md L4).
//!
//! Every period (`sample_rate / rate_hz` samples) a new target is drawn
//! uniformly from `[−1, 1]`, and the output moves from the previous target
//! to it along a cubic smoothstep `t²(3 − 2t)`. The curve is C¹ (no
//! corners, so no clicks or zipper on a modulated delay read), and its
//! slope peaks at `1.5·|Δ|/period ≤ 3/period`, so the largest per-sample
//! step of [`SmoothRandom::next`] is `3·depth·rate/sample_rate`.
//!
//! Output is `depth · y`, `|y| ≤ 1`: bounded by `depth`. The sequence is a
//! pure function of the seed, and [`SmoothRandom::reset`] restores it.

use crate::SimpleRng;

#[derive(Clone, Debug)]
pub struct SmoothRandom {
    seed: u64,
    state: u32,
    from: f32,
    to: f32,
    /// Position in the current segment, `[0, 1)`.
    phase: f32,
    /// Phase increment per sample (`rate / fs`).
    inc: f32,
    depth: f32,
}

/// One xorshift32 step from `state` (same generator as [`SimpleRng`]),
/// mapped to `[−1, 1]`.
#[inline]
fn draw(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    (x as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32
}

impl SmoothRandom {
    /// A modulator at `rate_hz` new targets per second, scaled by `depth`.
    pub fn new(seed: u64, rate_hz: f32, depth: f32, sample_rate: f32) -> Self {
        let mut s = Self {
            seed,
            state: 1,
            from: 0.0,
            to: 0.0,
            phase: 0.0,
            inc: 0.0,
            depth,
        };
        s.set_rate(rate_hz, sample_rate);
        s.reset();
        s
    }

    /// Change the rate. Takes effect smoothly: the current segment keeps
    /// its phase and continues at the new speed.
    pub fn set_rate(&mut self, rate_hz: f32, sample_rate: f32) {
        self.inc = (rate_hz.max(0.0) / sample_rate).min(0.5);
    }

    /// Change the depth (output scale). Not smoothed: callers that move it
    /// while running ramp it themselves.
    pub fn set_depth(&mut self, depth: f32) {
        self.depth = depth;
    }

    pub fn depth(&self) -> f32 {
        self.depth
    }

    /// Back to the freshly constructed sequence for this seed (rate and
    /// depth are kept).
    pub fn reset(&mut self) {
        // Derive the xorshift state the same way `SimpleRng::new` does.
        let mut rng = SimpleRng::new(self.seed);
        self.state = rng.next_u32() | 1;
        self.from = draw(&mut self.state) * 0.5;
        self.to = draw(&mut self.state);
        self.phase = 0.0;
    }

    /// The next sample, in `[−depth, depth]`.
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        let t = self.phase;
        let s = t * t * (3.0 - 2.0 * t);
        let y = self.from + (self.to - self.from) * s;
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.from = self.to;
            self.to = draw(&mut self.state);
        }
        y * self.depth
    }
}
