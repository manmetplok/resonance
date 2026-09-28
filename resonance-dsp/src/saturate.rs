//! Memoryless saturation curves with first-order antiderivative
//! antialiasing (ADAA).
//!
//! A [`Curve`] is a static transfer function `f(u)` together with its
//! closed-form antiderivative `F(u)`. [`Adaa1`] turns any curve into its
//! first-order ADAA form: each output is the exact average of `f` over the
//! segment the input travelled since the previous sample,
//! `(F(u[n]) − F(u[n−1])) / (u[n] − u[n−1])`. That continuous-time average
//! is an extra lowpass on the distortion products, so partials born above
//! Nyquist fold back far quieter than with the naive `f(u[n])`.
//!
//! # Conventions
//!
//! - Curves work on the **driven** input `u = drive · x`. Keep the drive
//!   outside the curve so ADAA stays valid while a drive smoother ramps.
//!   (The one exception is [`Curve::Console`], whose `drive` is part of the
//!   curve so that drive 0 is the identity.)
//! - Every curve passes through the origin with a finite slope.
//! - Maths is `f64`: the ADAA quotient subtracts two nearby antiderivative
//!   values, which `f32` cannot do accurately.
//! - Inputs are sanitised: NaN reads as 0, and `|u|` is capped at
//!   [`INPUT_LIMIT`], so no input produces a NaN or an infinity.
//!
//! # DC
//!
//! The odd curves ([`Curve::Tanh`], [`Curve::Console`], [`Curve::Clip`],
//! [`Curve::Inflator`]) add no DC to a symmetric signal. [`Curve::Tube`] and
//! [`Curve::Warm`] are asymmetric on purpose (that is where their even
//! harmonics come from) and **need a [`crate::DcBlocker`] after them**;
//! [`Curve::is_dc_safe`] reports which is which.
//!
//! # The linear part of ADAA
//!
//! On the small-signal (linear) part of a curve, first-order ADAA is the
//! two-tap average `(1 + z⁻¹)/2`: half a sample of delay and a gentle
//! lowpass (−0.7 dB at fs/8, −3 dB at fs/4, a null at Nyquist). At 1× that
//! is audible on the top octave; run the curve at 2× or 4× through the
//! [`crate::Oversampler`] (the lowpass then sits above the audio band), or
//! blend dry and wet through the same path. An identity curve (see
//! [`Curve::is_identity`]) bypasses ADAA entirely, so a stage at drive 0 is
//! bit-exact.
//!
//! # Oversampling and the aliasing floor
//!
//! Measured for a 5 kHz sine at 0 dBFS with +12 dB drive
//! (`tests/saturate.rs` pins these): ADAA inside the 4× IIR
//! [`crate::Oversampler`] keeps the strongest alias ≤ −90 dBc for
//! `Tanh`, `Tube`, `Warm`, `Console` and the soft and mid `Clip` shapes.
//! The hard clip (−73 dBc) and the `Inflator` (−80 dBc) have a slope
//! corner at full scale and need 8×, which the existing oversampler
//! provides by cascading: a 2× instance around a 4× one, each running its
//! filters at its own rate, still latency-free (hard clip −90 dBc,
//! inflator −105 dBc). The cost is the 4× stage running at twice the
//! base rate.
//!
//! ```
//! use resonance_dsp::{Adaa1, Curve, OversampleFactor, Oversampler};
//! let (mut outer, mut inner) = (Oversampler::new(), Oversampler::new());
//! outer.set_factor(OversampleFactor::X2);
//! inner.set_factor(OversampleFactor::X4);
//! let (mut adaa, curve) = (Adaa1::new(), Curve::Clip { shape: 1.0 });
//! let x = 0.9f32;
//! let mut mid = outer.upsample(4.0 * x);
//! for m in &mut mid[..2] {
//!     let mut hi = inner.upsample(*m);
//!     for v in &mut hi {
//!         *v = adaa.process(&curve, *v);
//!     }
//!     *m = inner.downsample(&hi);
//! }
//! let y = outer.downsample(&mid);
//! # let _ = y;
//! ```

/// Largest driven-input magnitude a curve evaluates (≈ +80 dB over unity).
/// Beyond it the input is clamped, which keeps every antiderivative (they
/// grow like `u²`) far from `f64` overflow.
pub const INPUT_LIMIT: f64 = 1.0e4;

/// Below this input step the ADAA quotient is ill-conditioned; the curve
/// is evaluated at the segment midpoint instead (error `O(Δu²)`).
const ADAA_EPS: f64 = 1.0e-5;

/// A memoryless saturation curve. The parameters are part of the curve,
/// so an [`Adaa1`] notices a change and recomputes its cached state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    /// `tanh(u)`. Odd harmonics only, smooth knee.
    Tanh,
    /// Biased asymmetric soft curve, `tanh(u + bias) − tanh(bias)`: the
    /// wavetable `Tube` shaper with the bias exposed. H2-dominant at low
    /// level, H3 catches up as the drive rises. `bias` 0 is [`Curve::Tanh`];
    /// the wavetable uses 0.5; clamped to ±2. Needs a DC blocker after it.
    Tube { bias: f32 },
    /// One-polarity "warm" curve with **only even** distortion:
    /// `u + (amount/2)·(√(1+u²) − 1)`. The distortion term is an even
    /// function, so a sine comes out with H2, H4, H6… and no odd
    /// harmonics. Negative half-waves are compressed (slope → 1 − a/2)
    /// and positive ones expanded (slope → 1 + a/2), the concave shape of
    /// a single-ended triode. `amount` is clamped to 0..=1; 0 is the
    /// identity. Needs a DC blocker after it.
    ///
    /// This is the PurestWarm idea made strictly even: saturating one
    /// polarity and leaving the other linear (PurestWarm itself) is half
    /// a symmetric saturator plus an even term, which measures H2 ≈ H3 +
    /// 6 dB, not even-only.
    Warm { amount: f32 },
    /// Console-style `sin` curve, `sin(k·u)/k` with `k = drive`, flattened
    /// at its peak (`|k·u| ≥ π/2`). Unity small-signal gain at any drive,
    /// and `drive` 0 is the identity (so the stage is transparent there).
    /// `drive` is clamped to 0..=4; the Airwindows channel curve is 1.
    Console { drive: f32 },
    /// Soft↔hard clipper with a ceiling of ±1 and unity gain below the
    /// knee. `shape` 0 is the softest (a quadratic knee from 0 to 2), 1 a
    /// hard clip at ±1; in between, the curve is linear up to `shape` and
    /// bends over a quadratic knee to reach 1 at `2 − shape`. Clamped to
    /// 0..=1.
    Clip { shape: f32 },
    /// Inflator-style odd polynomial (the RCInflator2 curve): for
    /// `y = |u| ≤ 1`, `A·y + B·y² + C·y³ − D·(y² − 2y³ + y⁴)` with
    /// `A = 1.5 + c`, `B = −2c`, `C = c − 0.5`, `D = 1/16 − c/4 + c²/4`,
    /// applied with the sign of `u`; it reaches 1 with zero slope at
    /// `|u| = 1` and is clamped to ±1 beyond. `curve` (`c`) is clamped to
    /// −0.5..=0.5 (the JSFX's −50…+50 %). Small-signal gain is `1.5 + c`,
    /// so blend it with the dry signal (the Inflator's "effect" control).
    Inflator { curve: f32 },
}

impl Curve {
    /// Whether the curve adds no DC to a symmetric input (it is odd).
    /// `false` means: put a [`crate::DcBlocker`] after it.
    pub fn is_dc_safe(&self) -> bool {
        match *self {
            Curve::Tube { bias } => clamp_bias(bias) == 0.0,
            Curve::Warm { amount } => not_positive(amount),
            _ => true,
        }
    }

    /// Whether the curve is exactly `f(u) = u` with these parameters.
    /// [`Adaa1`] passes such a curve straight through.
    pub fn is_identity(&self) -> bool {
        match *self {
            Curve::Warm { amount } => not_positive(amount),
            Curve::Console { drive } => not_positive(drive),
            _ => false,
        }
    }

    /// Small-signal gain `f'(0)`.
    pub fn slope_at_zero(&self) -> f64 {
        match *self {
            Curve::Tube { bias } => {
                let t = clamp_bias(bias).tanh();
                1.0 - t * t
            }
            Curve::Inflator { curve } => 1.5 + clamp_inflator(curve),
            _ => 1.0,
        }
    }

    /// The curve `f(u)`, without antialiasing.
    #[inline]
    pub fn eval(&self, u: f64) -> f64 {
        let u = sanitize(u);
        match *self {
            Curve::Tanh => u.tanh(),
            Curve::Tube { bias } => {
                let b = clamp_bias(bias);
                (u + b).tanh() - b.tanh()
            }
            Curve::Warm { amount } => {
                let a = clamp_unit(amount);
                u + 0.5 * a * sqrt1p_m1(u)
            }
            Curve::Console { drive } => {
                let k = clamp_console(drive);
                if k == 0.0 {
                    return u;
                }
                let v = k * u;
                if v.abs() >= std::f64::consts::FRAC_PI_2 {
                    v.signum() / k
                } else {
                    v.sin() / k
                }
            }
            Curve::Clip { shape } => {
                let (a, b) = clip_knee(shape);
                let y = u.abs();
                let m = if y <= a {
                    y
                } else if y < b {
                    let d = y - a;
                    y - d * d / (2.0 * (b - a))
                } else {
                    1.0
                };
                m.copysign(u)
            }
            Curve::Inflator { curve } => {
                let k = InflatorCoefs::new(curve);
                let y = u.abs();
                let m = if y >= 1.0 { 1.0 } else { k.g(y) };
                m.copysign(u)
            }
        }
    }

    /// The antiderivative `F(u)` (any constant of integration; only
    /// differences are used).
    #[inline]
    pub fn antiderivative(&self, u: f64) -> f64 {
        let u = sanitize(u);
        match *self {
            Curve::Tanh => ln_cosh(u),
            Curve::Tube { bias } => {
                let b = clamp_bias(bias);
                ln_cosh(u + b) - u * b.tanh()
            }
            Curve::Warm { amount } => {
                // ∫ √(1+u²) du = ½(u·√(1+u²) + asinh u).
                let a = clamp_unit(amount);
                let r = (1.0 + u * u).sqrt();
                0.5 * u * u + 0.5 * a * (0.5 * (u * r + u.asinh()) - u)
            }
            Curve::Console { drive } => {
                let k = clamp_console(drive);
                if k == 0.0 {
                    return 0.5 * u * u;
                }
                let y = u.abs();
                let knee = std::f64::consts::FRAC_PI_2 / k;
                if y <= knee {
                    // (1 − cos(k·y))/k², in the cancellation-free form.
                    let s = (0.5 * k * y).sin();
                    2.0 * s * s / (k * k)
                } else {
                    1.0 / (k * k) + (y - knee) / k
                }
            }
            Curve::Clip { shape } => {
                let (a, b) = clip_knee(shape);
                let y = u.abs();
                let w = b - a;
                if y <= a {
                    0.5 * y * y
                } else if y < b {
                    let d = y - a;
                    0.5 * y * y - d * d * d / (6.0 * w)
                } else {
                    0.5 * b * b - w * w / 6.0 + (y - b)
                }
            }
            Curve::Inflator { curve } => {
                let k = InflatorCoefs::new(curve);
                let y = u.abs();
                if y >= 1.0 {
                    k.big_g(1.0) + (y - 1.0)
                } else {
                    k.big_g(y)
                }
            }
        }
    }
}

/// First-order ADAA state for one channel. Feed it the driven input one
/// sample at a time; it never allocates and is safe on the audio thread.
///
/// Changing the curve (or its parameters) between samples is allowed: the
/// cached `F(u[n−1])` is keyed on the curve and recomputed when it
/// changes, so a parameter ramp never divides a stale antiderivative by a
/// tiny step.
#[derive(Clone, Copy, Debug)]
pub struct Adaa1 {
    x1: f64,
    f1: f64,
    cached: Option<Curve>,
}

impl Default for Adaa1 {
    fn default() -> Self {
        Self::new()
    }
}

impl Adaa1 {
    pub const fn new() -> Self {
        Self {
            x1: 0.0,
            f1: 0.0,
            cached: None,
        }
    }

    /// Forget the previous sample (the next output is `F(u)/u`, the
    /// average from the origin, which is continuous for a silent start).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// One antialiased sample of `curve` at driven input `u`.
    #[inline]
    pub fn process(&mut self, curve: &Curve, u: f32) -> f32 {
        let u0 = sanitize(u as f64);
        if curve.is_identity() {
            self.x1 = u0;
            self.cached = None;
            return u;
        }
        let u1 = self.x1;
        let f0 = curve.antiderivative(u0);
        let f1 = match self.cached {
            Some(c) if c == *curve => self.f1,
            _ => curve.antiderivative(u1),
        };
        let du = u0 - u1;
        let y = if du.abs() < ADAA_EPS {
            curve.eval(0.5 * (u0 + u1))
        } else {
            (f0 - f1) / du
        };
        self.x1 = u0;
        self.f1 = f0;
        self.cached = Some(*curve);
        y as f32
    }
}

#[inline]
fn sanitize(u: f64) -> f64 {
    if u.is_nan() {
        0.0
    } else {
        u.clamp(-INPUT_LIMIT, INPUT_LIMIT)
    }
}

/// `true` for zero, negative and NaN parameters.
#[inline]
fn not_positive(v: f32) -> bool {
    v.is_nan() || v <= 0.0
}

/// [`Curve::Tube`] bias, clamped to ±2 (NaN reads as 0).
#[inline]
fn clamp_bias(v: f32) -> f64 {
    if v.is_nan() {
        0.0
    } else {
        (v as f64).clamp(-2.0, 2.0)
    }
}

#[inline]
fn clamp_unit(v: f32) -> f64 {
    if v > 0.0 {
        (v as f64).min(1.0)
    } else {
        0.0
    }
}

#[inline]
fn clamp_console(v: f32) -> f64 {
    if v > 0.0 {
        (v as f64).min(4.0)
    } else {
        0.0
    }
}

#[inline]
fn clamp_inflator(v: f32) -> f64 {
    if v.is_nan() {
        0.0
    } else {
        (v as f64).clamp(-0.5, 0.5)
    }
}

/// `ln cosh(x)` without overflow: `|x| + ln(1 + e^{−2|x|}) − ln 2`.
#[inline]
fn ln_cosh(x: f64) -> f64 {
    let ax = x.abs();
    ax + (-2.0 * ax).exp().ln_1p() - std::f64::consts::LN_2
}

/// `√(1+u²) − 1` without cancellation near 0.
#[inline]
fn sqrt1p_m1(u: f64) -> f64 {
    let u2 = u * u;
    u2 / ((1.0 + u2).sqrt() + 1.0)
}

/// Knee start and end `(a, b)` of [`Curve::Clip`], `a + b = 2`.
#[inline]
fn clip_knee(shape: f32) -> (f64, f64) {
    let a = clamp_unit(shape);
    (a, 2.0 - a)
}

#[derive(Clone, Copy)]
struct InflatorCoefs {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
}

impl InflatorCoefs {
    #[inline]
    fn new(curve: f32) -> Self {
        let c = clamp_inflator(curve);
        Self {
            a: 1.5 + c,
            b: -2.0 * c,
            c: c - 0.5,
            d: 0.0625 - 0.25 * c + 0.25 * c * c,
        }
    }

    /// The polynomial on `0 ≤ y ≤ 1`.
    #[inline]
    fn g(&self, y: f64) -> f64 {
        let y2 = y * y;
        let y3 = y2 * y;
        self.a * y + self.b * y2 + self.c * y3 - self.d * (y2 - 2.0 * y3 + y2 * y2)
    }

    /// Its antiderivative from 0.
    #[inline]
    fn big_g(&self, y: f64) -> f64 {
        let y2 = y * y;
        let y3 = y2 * y;
        let y4 = y2 * y2;
        self.a * y2 / 2.0 + self.b * y3 / 3.0 + self.c * y4 / 4.0
            - self.d * (y3 / 3.0 - y4 / 2.0 + y4 * y / 5.0)
    }
}
