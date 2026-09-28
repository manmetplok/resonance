//! Jiles-Atherton magnetic hysteresis, the nonlinearity of a physically
//! based tape stage (warmth-width-depth.md §3.1 / §6.1, decision D2).
//!
//! # The model
//!
//! Magnetisation `M` follows the field `H` through the Jiles-Atherton
//! differential equation (Chowdhury, DAFx-19, eqs. 6–10 and 18):
//!
//! ```text
//!          (1−c)·δ_M·(M_an − M)
//!         ───────────────────────  +  c·(M_s/a)·L'(Q)
//!  dM      (1−c)·δ·k − α·(M_an − M)
//!  ──  =  ─────────────────────────────────────────────
//!  dH              1 − c·α·(M_s/a)·L'(Q)
//!
//!  Q = (H + α·M)/a,   M_an = M_s·L(Q),   L(x) = coth(x) − 1/x
//!  δ = sign(dH),      δ_M = 1 if δ and M_an − M share a sign, else 0
//! ```
//!
//! - `M_s` is the saturation magnetisation (the output ceiling),
//! - `a` shapes the anhysteretic curve (small `a` saturates sooner),
//! - `k` is the loop width (roughly the coercivity),
//! - `c` is the reversible fraction: `c → 1` is the anhysteretic curve
//!   with no loop at all, small `c` a wide, square loop,
//! - `α` is the inter-domain coupling.
//!
//! The paper writes the equation as `dM/dt = (dM/dH)·Ḣ` and integrates
//! it in time with `H` linearly interpolated between samples. Within one
//! sample step `H` is therefore a straight line, so that is exactly the
//! integral of `dM/dH` along `H` from `H[n−1]` to `H[n]`; this module
//! integrates in `H` directly. The two are the same scheme, but the `H`
//! form needs no `Ḣ` state and makes plain that the model is
//! rate-independent: the sample rate only sets how finely each step
//! resolves the path.
//!
//! # Controls
//!
//! [`JaParams::from_controls`] maps the three musical controls the Chow
//! Tape approach exposes onto the physical constants, with `k` and `α`
//! fixed:
//!
//! - **drive** (a linear gain `G`) → `a = M_s / (3·G)`, so the loop's
//!   anhysteretic small-signal slope `M_s/(3a)` is `G`;
//! - **sat** (0..1) → `M_s`, from [`M_S_MAX`] down to [`M_S_MIN`]: more
//!   `sat` lowers the ceiling at the same slope, so the curve saturates
//!   sooner;
//! - **bias** (0..1) → `c`, from [`C_MIN`] (under-biased: a wide loop,
//!   dirty, quiet signals lose level) to [`C_MAX`] (over-biased: close to
//!   the clean anhysteretic curve). This is tape bias in the recording
//!   sense, not asymmetry: the model is symmetric and makes odd
//!   harmonics only.
//!
//! The constants are this module's own voicing in normalised units (a
//! full-scale input is `H = 1`), not the paper's ferric-oxide values in
//! A/m.
//!
//! # Solvers
//!
//! [`HysteresisSolver`] picks the integrator per step:
//!
//! - **RK2** (Heun): two evaluations per sub-step, the cheapest;
//! - **RK4**: four evaluations, the paper's choice and the default;
//! - **NR**: implicit trapezoidal rule solved by Newton-Raphson (up to
//!   [`NR_MAX_ITERS`] iterations of three evaluations each). A-stable, so
//!   it takes the longest sub-steps.
//!
//! # Stability
//!
//! The irreversible term relaxes `M` toward `M_an` over a field span of
//! about `k`, which makes the equation stiff for steps much larger than
//! `k` (loud high-frequency material, low stage rates). Three guards keep
//! every solver finite and bounded, at every drive and on any input:
//!
//! - each step is split into up to [`MAX_SUBSTEPS`] sub-steps no longer
//!   than the solver's stable span (a multiple of `k`), which bounds the
//!   per-sample cost for the audio thread;
//! - the derivative is clamped: both denominators have floors (the
//!   irreversible one can cross zero when `α·|M_an − M|` reaches
//!   `(1−c)·k`), `dM/dH` is held to `[0, `[`DERIV_MAX`]`]` — it is never
//!   negative in this model — and a non-finite value reads as 0;
//! - `M` is clamped to `±M_s` after every sub-step.
//!
//! [`langevin`] and [`langevin_deriv`] switch to their Taylor series near
//! zero, where `coth(x) − 1/x` cancels catastrophically.
//!
//! # RT safety
//!
//! [`Hysteresis`] is `Copy`-sized state (no heap), and
//! [`Hysteresis::process`] allocates nothing, takes no locks and runs a
//! bounded number of evaluations.

/// Loop width `k`, in normalised field units (full scale = 1).
pub const K: f64 = 0.05;
/// Inter-domain coupling `α`. Small enough that `α·2·M_S_MAX` stays
/// below `(1 − C_MAX)·K`, so the irreversible denominator never crosses
/// zero at any control setting (the floor below is a second guard).
pub const ALPHA: f64 = 1.0e-4;
/// `M_s` at `sat` = 0 and 1.
pub const M_S_MAX: f64 = 1.5;
pub const M_S_MIN: f64 = 0.5;
/// `c` at `bias` = 0 and 1.
pub const C_MIN: f64 = 0.5;
pub const C_MAX: f64 = 0.95;
/// Most sub-steps one [`Hysteresis::process`] call takes.
pub const MAX_SUBSTEPS: u32 = 8;
/// Most Newton iterations per NR sub-step.
pub const NR_MAX_ITERS: u32 = 8;
/// Upper clamp of `dM/dH`.
pub const DERIV_MAX: f64 = 1.0e4;

/// Below this `|x|` the Langevin function and its derivative use their
/// series.
const SERIES_X: f64 = 1.0e-2;

/// The Langevin function `L(x) = coth(x) − 1/x`.
#[inline]
pub fn langevin(x: f64) -> f64 {
    langevin_pair(x).0
}

/// `L'(x) = 1/x² − 1/sinh²(x)`.
#[inline]
pub fn langevin_deriv(x: f64) -> f64 {
    langevin_pair(x).1
}

/// `(L(x), L'(x))` from one `exp`: with `e = e^{−2|x|}`,
/// `coth|x| = (1 + e)/(1 − e)` and `1/sinh²x = 4e/(1 − e)²`, where
/// `1 − e` comes from `expm1` so it keeps its precision near zero.
#[inline]
pub fn langevin_pair(x: f64) -> (f64, f64) {
    let ax = x.abs();
    if ax < SERIES_X {
        // L = x/3 − x³/45 + 2x⁵/945,  L' = 1/3 − x²/15 + 2x⁴/189.
        let x2 = x * x;
        let l = x * (1.0 / 3.0 - x2 * (1.0 / 45.0 - x2 * (2.0 / 945.0)));
        let dl = 1.0 / 3.0 - x2 * (1.0 / 15.0 - x2 * (2.0 / 189.0));
        return (l, dl);
    }
    let e = (-2.0 * ax).exp();
    let one_minus_e = -(-2.0 * ax).exp_m1();
    let coth = (1.0 + e) / one_minus_e;
    let inv_x = 1.0 / ax;
    let l = (coth - inv_x).copysign(x);
    let dl = inv_x * inv_x - 4.0 * e / (one_minus_e * one_minus_e);
    (l, dl.max(0.0))
}

/// The integrator a [`Hysteresis`] uses. The discriminant is a plugin
/// parameter's plain value, so the order is part of saved state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum HysteresisSolver {
    Rk2 = 0,
    #[default]
    Rk4 = 1,
    NewtonRaphson = 2,
}

impl HysteresisSolver {
    pub const LABELS: [&'static str; 3] = ["RK2", "RK4", "NR"];
    pub const ALL: [HysteresisSolver; 3] = [Self::Rk2, Self::Rk4, Self::NewtonRaphson];

    pub fn from_int(v: i32) -> Self {
        match v {
            0 => Self::Rk2,
            2 => Self::NewtonRaphson,
            _ => Self::Rk4,
        }
    }

    /// Longest sub-step, as a multiple of `k`, inside the solver's
    /// stability region for the irreversible relaxation (real-axis
    /// limits: 2 for RK2, ≈ 2.79 for RK4; NR is A-stable and is limited
    /// for accuracy only).
    pub fn span_in_k(self) -> f64 {
        match self {
            Self::Rk2 => 1.5,
            Self::Rk4 => 2.5,
            Self::NewtonRaphson => 4.0,
        }
    }
}

/// The Jiles-Atherton constants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JaParams {
    pub m_s: f64,
    pub a: f64,
    pub alpha: f64,
    pub k: f64,
    pub c: f64,
}

impl JaParams {
    /// Map drive gain `G` (linear, > 0), `sat` and `bias` (0..1) to the
    /// constants (see the module docs). Out-of-range or NaN inputs are
    /// clamped.
    pub fn from_controls(gain: f64, sat: f64, bias: f64) -> Self {
        let unit = |v: f64| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
        let g = if gain.is_finite() { gain.clamp(1.0e-3, 1.0e3) } else { 1.0 };
        let m_s = M_S_MAX - (M_S_MAX - M_S_MIN) * unit(sat);
        Self {
            m_s,
            a: m_s / (3.0 * g),
            alpha: ALPHA,
            k: K,
            c: C_MIN + (C_MAX - C_MIN) * unit(bias),
        }
    }

    /// Slope of the anhysteretic curve at the origin, `dM_an/dH`,
    /// including the mean-field feedback: `χ/(1 − α·χ)` with
    /// `χ = M_s/(3a)`. What the loop's centre line does to a small
    /// signal; a caller divides by it to normalise the level.
    pub fn anhysteretic_slope(&self) -> f64 {
        let chi = self.m_s / (3.0 * self.a);
        chi / (1.0 - self.alpha * chi).max(0.1)
    }

    /// The initial susceptibility from the demagnetised state,
    /// `c·χ/(1 − α·c·χ)`: the gain a very quiet signal sees.
    pub fn initial_slope(&self) -> f64 {
        let chi = self.c * self.m_s / (3.0 * self.a);
        chi / (1.0 - self.alpha * chi).max(0.1)
    }
}

impl Default for JaParams {
    fn default() -> Self {
        Self::from_controls(1.0, 0.5, 0.5)
    }
}

/// Constants derived from a [`JaParams`], cached per parameter change.
#[derive(Clone, Copy, Debug)]
struct Derived {
    inv_a: f64,
    ms_over_a: f64,
    one_minus_c: f64,
    /// `(1 − c)·k`.
    irr_k: f64,
    /// Floor of the irreversible denominator.
    irr_floor: f64,
    /// Longest sub-step in `H`.
    max_step: f64,
}

/// One channel of Jiles-Atherton hysteresis: `H` in, `M` out.
#[derive(Clone, Copy, Debug)]
pub struct Hysteresis {
    p: JaParams,
    d: Derived,
    solver: HysteresisSolver,
    m: f64,
    h_prev: f64,
}

impl Hysteresis {
    pub fn new(params: JaParams, solver: HysteresisSolver) -> Self {
        let mut s = Self {
            p: params,
            d: Derived {
                inv_a: 0.0,
                ms_over_a: 0.0,
                one_minus_c: 0.0,
                irr_k: 0.0,
                irr_floor: 0.0,
                max_step: 0.0,
            },
            solver,
            m: 0.0,
            h_prev: 0.0,
        };
        s.set_params(params);
        s
    }

    pub fn params(&self) -> JaParams {
        self.p
    }

    pub fn solver(&self) -> HysteresisSolver {
        self.solver
    }

    /// Change the constants. Keeps `M` (clamped to the new `±M_s`), so a
    /// smoothed drive moves the loop without a jump.
    pub fn set_params(&mut self, p: JaParams) {
        let sane = |v: f64, lo: f64, hi: f64, dflt: f64| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                dflt
            }
        };
        let p = JaParams {
            m_s: sane(p.m_s, 1.0e-6, 1.0e6, 1.0),
            a: sane(p.a, 1.0e-9, 1.0e9, 1.0 / 3.0),
            alpha: sane(p.alpha, 0.0, 1.0, ALPHA),
            k: sane(p.k, 1.0e-9, 1.0e9, K),
            c: sane(p.c, 0.0, 1.0, 0.5),
        };
        self.p = p;
        let one_minus_c = 1.0 - p.c;
        let irr_k = one_minus_c * p.k;
        self.d = Derived {
            inv_a: 1.0 / p.a,
            ms_over_a: p.m_s / p.a,
            one_minus_c,
            irr_k,
            irr_floor: 0.25 * irr_k,
            max_step: self.solver.span_in_k() * p.k,
        };
        self.m = self.m.clamp(-p.m_s, p.m_s);
    }

    pub fn set_solver(&mut self, solver: HysteresisSolver) {
        self.solver = solver;
        self.d.max_step = solver.span_in_k() * self.p.k;
    }

    /// Back to the demagnetised state at `H = 0`.
    pub fn reset(&mut self) {
        self.m = 0.0;
        self.h_prev = 0.0;
    }

    /// The current magnetisation.
    pub fn magnetisation(&self) -> f64 {
        self.m
    }

    /// `dM/dH` at `(m, h)` for a field moving in direction `delta`
    /// (±1), with every guard of the module docs applied.
    #[inline]
    pub fn slope(&self, m: f64, h: f64, delta: f64) -> f64 {
        let p = &self.p;
        let d = &self.d;
        let q = (h + p.alpha * m) * d.inv_a;
        let (l, dl) = langevin_pair(q);
        let diff = p.m_s * l - m;
        let dman = d.ms_over_a * dl;
        let irr = if diff * delta > 0.0 && d.irr_k > 0.0 {
            let ad = diff.abs();
            d.one_minus_c * ad / (d.irr_k - p.alpha * ad).max(d.irr_floor)
        } else {
            0.0
        };
        let den = (1.0 - p.alpha * p.c * dman).max(0.1);
        let g = (irr + p.c * dman) / den;
        if g.is_finite() {
            g.clamp(0.0, DERIV_MAX)
        } else {
            0.0
        }
    }

    /// Advance to field `h` and return the new `M`. A non-finite `h`
    /// reads as 0.
    #[inline]
    pub fn process(&mut self, h: f64) -> f64 {
        let h = if h.is_finite() { h } else { 0.0 };
        let span = h - self.h_prev;
        if span != 0.0 {
            let n = (span.abs() / self.d.max_step).ceil().clamp(1.0, MAX_SUBSTEPS as f64);
            let dh = span / n;
            let delta = if span > 0.0 { 1.0 } else { -1.0 };
            let lim = self.p.m_s;
            let mut m = self.m;
            let mut h0 = self.h_prev;
            for _ in 0..n as u32 {
                m = match self.solver {
                    HysteresisSolver::Rk2 => self.step_rk2(m, h0, dh, delta),
                    HysteresisSolver::Rk4 => self.step_rk4(m, h0, dh, delta),
                    HysteresisSolver::NewtonRaphson => self.step_nr(m, h0, dh, delta),
                };
                m = if m.is_finite() { m.clamp(-lim, lim) } else { 0.0 };
                h0 += dh;
            }
            self.m = m;
        }
        self.h_prev = h;
        self.m
    }

    #[inline]
    fn step_rk2(&self, m: f64, h0: f64, dh: f64, delta: f64) -> f64 {
        let k1 = dh * self.slope(m, h0, delta);
        let k2 = dh * self.slope(m + k1, h0 + dh, delta);
        m + 0.5 * (k1 + k2)
    }

    #[inline]
    fn step_rk4(&self, m: f64, h0: f64, dh: f64, delta: f64) -> f64 {
        let hm = h0 + 0.5 * dh;
        let k1 = dh * self.slope(m, h0, delta);
        let k2 = dh * self.slope(m + 0.5 * k1, hm, delta);
        let k3 = dh * self.slope(m + 0.5 * k2, hm, delta);
        let k4 = dh * self.slope(m + k3, h0 + dh, delta);
        m + (k1 + 2.0 * k2 + 2.0 * k3 + k4) / 6.0
    }

    /// Implicit trapezoidal step, `m₁ = m₀ + dh/2·(g(m₀, h₀) + g(m₁, h₁))`,
    /// solved by Newton-Raphson from an explicit Euler guess, with a
    /// central-difference Jacobian.
    #[inline]
    fn step_nr(&self, m0: f64, h0: f64, dh: f64, delta: f64) -> f64 {
        let h1 = h0 + dh;
        let lim = self.p.m_s;
        let g0 = self.slope(m0, h0, delta);
        let mut m = (m0 + dh * g0).clamp(-lim, lim);
        let eps = 1.0e-7 * lim;
        let tol = 1.0e-12 * lim;
        for _ in 0..NR_MAX_ITERS {
            let f = m - m0 - 0.5 * dh * (g0 + self.slope(m, h1, delta));
            let dg = (self.slope(m + eps, h1, delta) - self.slope(m - eps, h1, delta)) / (2.0 * eps);
            let fp = 1.0 - 0.5 * dh * dg;
            if !(fp.abs() > 1.0e-12) || !f.is_finite() {
                break;
            }
            let step = f / fp;
            m = (m - step).clamp(-lim, lim);
            if step.abs() < tol {
                break;
            }
        }
        m
    }
}

impl Default for Hysteresis {
    fn default() -> Self {
        Self::new(JaParams::default(), HysteresisSolver::default())
    }
}
