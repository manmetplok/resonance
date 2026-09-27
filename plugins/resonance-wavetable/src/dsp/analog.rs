//! Analog instability: random oscillator start phase, slow pitch drift and
//! small per-note spreads of filter cutoff and oscillator level.
//!
//! Everything random here is drawn from [`AnalogRng`], a plain xorshift32
//! that lives by value in the engine and in each voice — no allocation, no
//! shared state, no syscalls, and a fixed seed, so an offline bounce renders
//! the same bits every time.
//!
//! The random *draws* never depend on the knob positions: a note-on consumes
//! the same numbers whether `analog` is 0 or 1, and the knobs only scale what
//! was drawn. At the default of 0 every scaled term is an exact `±0.0` (an
//! add) or `1.0` (a multiply), so the render path is bit-identical to a synth
//! without this module.

/// Peak pitch deviation of the drift walk at `analog` = 1, in cents.
pub const DRIFT_MAX_CENTS: f32 = 6.0;

/// Peak per-note filter cutoff offset at `analog` = 1, in octaves
/// (about ±11 %).
pub const CUTOFF_SPREAD_OCT: f32 = 0.15;

/// Peak per-note oscillator level offset at `analog` = 1, as a fraction of
/// the level (about ±0.7 dB).
pub const LEVEL_SPREAD: f32 = 0.08;

/// The drift walk advances once every this many samples.
///
/// Every step moves the pitch, which invalidates the voice's cached
/// `OscSetup` and costs a rebuild (`exp2`, `log2`, `sin`/`cos` per unison
/// per oscillator). The walk's fastest component is ~1 Hz and its per-step
/// pitch change a small fraction of a cent, so 64 samples (750 Hz at 48 kHz)
/// is far past what the ear resolves while keeping the rebuild at a quarter
/// of the control-tick rate. A power of two and a multiple of the
/// control-rate interval, so it lands on that grid.
pub const DRIFT_INTERVAL: u32 = 64;

/// Cut-off of the one-pole that smooths the walk towards its target, Hz.
const DRIFT_SMOOTH_HZ: f32 = 0.6;

/// How long the walk holds a target before drawing the next, seconds.
const DRIFT_HOLD_MIN_S: f32 = 0.2;
const DRIFT_HOLD_MAX_S: f32 = 1.2;

/// Minimal xorshift32. `Copy` so it can sit inside `Voice` (which is
/// `Clone`) — `resonance_dsp::SimpleRng` is neither.
#[derive(Clone, Copy)]
pub struct AnalogRng {
    state: u32,
}

impl AnalogRng {
    pub fn new(seed: u32) -> Self {
        // `| 1`: zero is xorshift's fixed point.
        Self { state: seed | 1 }
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state
    }

    /// Uniform in `[0, 1)`.
    #[inline]
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    /// Uniform in `[-1, 1)`.
    #[inline]
    pub fn bipolar(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

impl Default for AnalogRng {
    fn default() -> Self {
        Self::new(1)
    }
}

/// Block-constant coefficients of the drift walk, resolved from the sample
/// rate.
#[derive(Clone, Copy)]
pub struct DriftCoeffs {
    /// One-pole smoothing coefficient per walk step.
    pub smooth: f32,
    /// Hold time range in walk steps.
    pub hold_min: u32,
    pub hold_span: u32,
}

impl DriftCoeffs {
    pub fn for_sample_rate(sample_rate: f32) -> Self {
        let step_rate = sample_rate / DRIFT_INTERVAL as f32;
        let smooth = 1.0 - (-std::f32::consts::TAU * DRIFT_SMOOTH_HZ / step_rate).exp();
        let hold_min = (DRIFT_HOLD_MIN_S * step_rate) as u32;
        let hold_max = (DRIFT_HOLD_MAX_S * step_rate) as u32;
        Self {
            smooth,
            hold_min: hold_min.max(1),
            hold_span: hold_max.saturating_sub(hold_min).max(1),
        }
    }
}

/// A bounded, smoothed random walk in `[-1, 1]`: a sample-and-hold of random
/// targets at random intervals, followed by a slow one-pole. The one-pole's
/// output is a convex combination of values in `[-1, 1]`, so it can never
/// leave that range whatever the step count.
#[derive(Clone, Copy, Default)]
pub struct DriftWalk {
    /// Current walk position, `[-1, 1]`.
    pub value: f32,
    target: f32,
    hold: u32,
}

impl DriftWalk {
    /// Start a fresh walk at a random position, so a note begins somewhere
    /// off-centre rather than every note starting in tune.
    #[inline]
    pub fn start(&mut self, rng: &mut AnalogRng, coeffs: &DriftCoeffs) {
        self.value = rng.bipolar();
        self.target = rng.bipolar();
        self.hold = coeffs.hold_min + rng.next_u32() % coeffs.hold_span;
    }

    /// Advance one walk step.
    #[inline]
    pub fn step(&mut self, rng: &mut AnalogRng, coeffs: &DriftCoeffs) {
        if self.hold == 0 {
            self.target = rng.bipolar();
            self.hold = coeffs.hold_min + rng.next_u32() % coeffs.hold_span;
        } else {
            self.hold -= 1;
        }
        self.value += (self.target - self.value) * coeffs.smooth;
    }
}
