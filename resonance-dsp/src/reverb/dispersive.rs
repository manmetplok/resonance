//! Dispersive allpass: a cascade of identical first-order allpasses with a
//! high coefficient, the chirp generator of a spring reverb
//! (reverb-algorithms.md §4.3; Välimäki, Parker & Abel's parametric spring
//! model).
//!
//! Each stage is
//!
//! ```text
//!   H(z) = (−a + z⁻¹) / (1 − a·z⁻¹)
//! ```
//!
//! flat in magnitude for any `|a| < 1`, with the pole at `z = a`. Its group
//! delay, in samples, is
//!
//! ```text
//!   τ(ω) = (1 − a²) / (1 − 2a·cos ω + a²)
//! ```
//!
//! so **for a positive coefficient the group delay is largest at DC,
//! `(1 + a)/(1 − a)`, and falls monotonically to `(1 − a)/(1 + a)` at
//! Nyquist**: low frequencies leave the cascade later than high ones. A
//! cascade of `M` stages multiplies that by `M`, which turns an impulse
//! into a falling chirp (the treble arrives first, the bass trails), and a
//! feedback loop round the cascade repeats it with the dispersion growing
//! on every pass, as on a real spring. A coefficient towards 1 piles the
//! delay into the lowest frequencies (a long, deep chirp); towards 0 the
//! cascade becomes a plain `M`-sample delay. A negative coefficient
//! mirrors it (high frequencies trail).
//!
//! The implementation keeps one state per stage
//! (`y = −a·x + s`, `s ← x + a·y`), so a stage is two multiplies and two
//! adds. The coefficient may move on running audio (each stage stays
//! stable and bounded for `|a| < 1`; it is clamped to `±0.999`); a change
//! of the stage count is a discontinuity in the group delay and is meant
//! for silence or a crossfade. Everything is allocated in
//! [`DispersiveAllpass::new`]; [`DispersiveAllpass::clear`] returns the
//! cascade to its freshly built state bit-exactly.

/// The largest coefficient magnitude accepted (the pole stays inside the
/// unit circle with margin).
pub const MAX_DISPERSION_COEFFICIENT: f32 = 0.999;

#[derive(Clone, Debug)]
pub struct DispersiveAllpass {
    state: Vec<f32>,
    stages: usize,
    a: f32,
}

impl DispersiveAllpass {
    /// A cascade with room for `max_stages`, running `stages` of them, at
    /// coefficient `a`.
    pub fn new(max_stages: usize, stages: usize, a: f32) -> Self {
        let max_stages = max_stages.max(1);
        Self {
            state: vec![0.0; max_stages],
            stages: stages.min(max_stages),
            a: clamp_coefficient(a),
        }
    }

    /// Change the coefficient (clamped to `±MAX_DISPERSION_COEFFICIENT`).
    pub fn set_coefficient(&mut self, a: f32) {
        self.a = clamp_coefficient(a);
    }

    pub fn coefficient(&self) -> f32 {
        self.a
    }

    /// Change the number of stages in use (clamped to the capacity). The
    /// states of stages that drop out are zeroed, so re-adding them later
    /// starts from silence.
    pub fn set_stages(&mut self, stages: usize) {
        let stages = stages.min(self.state.len());
        for s in &mut self.state[stages.min(self.stages)..] {
            *s = 0.0;
        }
        self.stages = stages;
    }

    pub fn stages(&self) -> usize {
        self.stages
    }

    pub fn max_stages(&self) -> usize {
        self.state.len()
    }

    /// One sample through every stage in use.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let a = self.a;
        let mut v = x;
        for s in &mut self.state[..self.stages] {
            let y = *s - a * v;
            *s = v + a * y;
            v = y;
        }
        v
    }

    /// One sample through this cascade (`x`) and `other` (`y`) at once,
    /// stage by stage. Bit-identical to two [`DispersiveAllpass::process`]
    /// calls, but the two dependency chains interleave, which roughly
    /// halves the cost of a stereo pair on an out-of-order CPU. Stages past
    /// the shorter cascade's count run on the longer one alone.
    #[inline]
    pub fn process_pair(&mut self, other: &mut Self, x: f32, y: f32) -> (f32, f32) {
        let (a, b) = (self.a, other.a);
        let (mut u, mut v) = (x, y);
        let common = self.stages.min(other.stages);
        let (sa, ta) = self.state[..self.stages].split_at_mut(common);
        let (sb, tb) = other.state[..other.stages].split_at_mut(common);
        for (s, t) in sa.iter_mut().zip(sb.iter_mut()) {
            let yu = *s - a * u;
            let yv = *t - b * v;
            *s = u + a * yu;
            *t = v + b * yv;
            u = yu;
            v = yv;
        }
        for s in ta {
            let yu = *s - a * u;
            *s = u + a * yu;
            u = yu;
        }
        for t in tb {
            let yv = *t - b * v;
            *t = v + b * yv;
            v = yv;
        }
        (u, v)
    }

    /// Back to silence (the coefficient and stage count are kept).
    pub fn clear(&mut self) {
        self.state.iter_mut().for_each(|s| *s = 0.0);
    }

    /// Group delay of the whole cascade at `freq_hz`, in samples.
    pub fn group_delay(&self, freq_hz: f32, sample_rate: f32) -> f32 {
        self.stages as f32 * stage_group_delay(self.a, freq_hz, sample_rate)
    }
}

/// Group delay of one stage with coefficient `a` at `freq_hz`, samples.
pub fn stage_group_delay(a: f32, freq_hz: f32, sample_rate: f32) -> f32 {
    let a = clamp_coefficient(a) as f64;
    let w = std::f64::consts::TAU * freq_hz as f64 / sample_rate as f64;
    ((1.0 - a * a) / (1.0 - 2.0 * a * w.cos() + a * a)) as f32
}

fn clamp_coefficient(a: f32) -> f32 {
    if a.is_finite() {
        a.clamp(-MAX_DISPERSION_COEFFICIENT, MAX_DISPERSION_COEFFICIENT)
    } else {
        0.0
    }
}
