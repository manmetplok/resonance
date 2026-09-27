//! Per-oscillator phase warp: Serum-style phase distortion applied to the
//! read phase before the table lookup.
//!
//! A warp is a map `p -> p'` on one cycle of the oscillator phase (plus, for
//! [`WarpMode::Formant`], a gain). The oscillator still advances its phase
//! linearly; only the position it *reads* the table at moves. Three facts
//! about each map drive everything else in this module:
//!
//! * **Its steepest slope.** Where `dp'/dp = k` the table is swept `k` times
//!   faster than the fundamental, so every partial it holds sounds `k` times
//!   higher. [`Warp::bandwidth`] is that worst-case `k`, and the render path
//!   multiplies the mip-selection frequency by it: the oscillator reads a
//!   level band-limited for `k * f` rather than `f`, which keeps the
//!   stretched partials under Nyquist at the price of the top end at gentle
//!   settings. That is the conservative choice the aliasing tests pin.
//! * **Whether it jumps at the cycle wrap.** Bend and PWM map `0 -> 0` and
//!   `1 -> 1`, which is the same point of a cyclic table, so the output stays
//!   continuous. Mirror and Formant end the cycle somewhere else, which is a
//!   step discontinuity once per period — a hard-sync edge. No mip level can
//!   band-limit a step, so the render path corrects it with a polyBLEP
//!   instead ([`blep_split`]); the step's height is fixed per setup and
//!   precomputed at control rate.
//! * **Whether it is stepped.** Quantize holds the phase on `N` steps per
//!   cycle — a staircase, i.e. a step every `1/N` of a period. Those steps
//!   are the effect, so they are kept, but each one is polyBLEP-corrected
//!   like the sync edge so the grit stays harmonic instead of folding.
//!
//! Every mode resolves to [`WarpMode::Off`] at an amount of zero, and the
//! render path then takes exactly the unwarped read: an idle warp control is
//! bit-identical to no warp at all.
//!
//! All the per-sample work here is a handful of multiplies, at most one
//! divide and a `floor`: no transcendental math. The `exp2` a Bend or
//! Quantize setting needs runs once per control-rate re-plan, in
//! [`Warp::resolve`].

/// The warp applied to one oscillator's read phase. Values are the
/// `oscN_warp_mode` parameter's integers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum WarpMode {
    #[default]
    Off = 0,
    /// Power-curve bend, symmetric about the half cycle. Positive amounts
    /// squeeze the table's content toward the middle of the period,
    /// negative amounts push it out to the edges.
    Bend = 1,
    /// Fold the cycle: at full amount the table is read forward over the
    /// first half period and backward over the second.
    Mirror = 2,
    /// Phase squeeze: the whole table is read in the first `w` of the
    /// period and its end value held for the rest, which turns a saw into a
    /// pulse-like shape of width `w`.
    Pwm = 3,
    /// Window sync: the table is read `r` times per period under a window,
    /// which moves a formant up to `r` times the fundamental.
    Formant = 4,
    /// Phase quantize ("bitcrush"): the read phase is held on `N` steps per
    /// cycle, from 256 at a small amount down to 4, each step reading the
    /// table at its centre.
    Quantize = 5,
}

impl WarpMode {
    /// Display names, indexed by the parameter's integer value. The
    /// parameter declares these as its choices, so the host, the MCP
    /// control surface and the editor all read the same table.
    pub const LABELS: [&'static str; 6] = ["Off", "Bend", "Mirror", "PWM", "Formant", "Quant"];

    pub fn from_int(v: i32) -> Self {
        match v {
            1 => Self::Bend,
            2 => Self::Mirror,
            3 => Self::Pwm,
            4 => Self::Formant,
            5 => Self::Quantize,
            _ => Self::Off,
        }
    }

    pub fn label(self) -> &'static str {
        Self::LABELS[self as usize]
    }
}

/// Bend's curve at full amount: the steepest slope is `2^BEND_OCTAVES`, so
/// the mip selection darkens by up to this many octaves.
const BEND_OCTAVES: f32 = 3.0;
/// PWM's narrowest width at full amount is `1 - PWM_MAX_SQUEEZE` of the
/// period (slope 10).
const PWM_MAX_SQUEEZE: f64 = 0.9;
/// Formant's read ratio at full amount.
const FORMANT_MAX_RATIO: f64 = 8.0;
/// Quantize's step count spans `2^QUANT_MAX_LOG2` (just above zero) down to
/// `2^QUANT_MIN_LOG2` (full amount). Not down to 2: two samples of a
/// symmetric frame (sine, triangle, saw) land on its zero crossings and
/// read silence.
const QUANT_MAX_LOG2: f32 = 8.0;
const QUANT_MIN_LOG2: f32 = 2.0;

/// A warp resolved for one oscillator setup: the mode plus its
/// amount-derived coefficient, so the per-sample map is arithmetic only.
///
/// Resolved at control rate (it depends on the warp-amount modulation
/// destination) and cached in [`OscSetup`](crate::dsp::voice::OscSetup).
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Warp {
    mode: WarpMode,
    /// |amount| in 0..=1 (Mirror, Formant window depth).
    depth: f64,
    /// Mode coefficient: Bend's curve `g`, PWM's width `w`, Formant's ratio
    /// `r`, Quantize's step count `N`.
    k: f64,
}

impl Warp {
    /// Resolve `mode` at `amount` (-1..=1; the unipolar modes use its
    /// magnitude). An amount of zero resolves to [`WarpMode::Off`].
    pub fn resolve(mode: WarpMode, amount: f32) -> Self {
        let amount = amount.clamp(-1.0, 1.0);
        if mode == WarpMode::Off || amount == 0.0 {
            return Self::default();
        }
        let depth = amount.abs() as f64;
        let k = match mode {
            WarpMode::Off => 0.0,
            WarpMode::Bend => (BEND_OCTAVES * amount).exp2() as f64,
            WarpMode::Mirror => 0.0,
            WarpMode::Pwm => 1.0 - PWM_MAX_SQUEEZE * depth,
            WarpMode::Formant => 1.0 + (FORMANT_MAX_RATIO - 1.0) * depth,
            WarpMode::Quantize => {
                let log2 = QUANT_MAX_LOG2 + (QUANT_MIN_LOG2 - QUANT_MAX_LOG2) * depth as f32;
                log2.exp2() as f64
            }
        };
        Self { mode, depth, k }
    }

    #[inline]
    pub fn mode(&self) -> WarpMode {
        self.mode
    }

    #[inline]
    pub fn is_off(&self) -> bool {
        self.mode == WarpMode::Off
    }

    /// The steepest slope of the phase map, `max |dp'/dp|` over the cycle:
    /// the factor by which the warp can raise the frequency of any partial
    /// in the table. The mip selection is biased by exactly this.
    pub fn bandwidth(&self) -> f32 {
        let b = match self.mode {
            WarpMode::Off | WarpMode::Quantize => 1.0,
            // `f(x) = x / (x + (1 - x) g)` has slope `1/g` at 0 and `g` at 1.
            WarpMode::Bend => self.k.max(1.0 / self.k),
            // Slopes `1 + a` (first half) and `|1 - 3a|` (second), and
            // `|1 - 3a| <= 1 + a` on 0..=1.
            WarpMode::Mirror => 1.0 + self.depth,
            WarpMode::Pwm => 1.0 / self.k,
            WarpMode::Formant => self.k,
        };
        b as f32
    }

    /// True when the map ends the cycle somewhere other than where it
    /// starts, i.e. the output steps once per period at the phase wrap.
    #[inline]
    pub fn jumps_at_wrap(&self) -> bool {
        matches!(self.mode, WarpMode::Mirror | WarpMode::Formant)
    }

    /// Steps per cycle for [`WarpMode::Quantize`], else `None`.
    #[inline]
    pub fn steps(&self) -> Option<f64> {
        (self.mode == WarpMode::Quantize).then_some(self.k)
    }

    /// Map an oscillator phase `p` (0..=1) to the table read phase (0..=1)
    /// and an output gain. A read phase of exactly 1.0 is the table's
    /// wrap point, which the reader handles like 0.0.
    #[inline]
    pub fn apply(&self, p: f64) -> (f64, f32) {
        match self.mode {
            WarpMode::Off => (p, 1.0),
            WarpMode::Bend => {
                // The rational curve rather than `x.powf(e)`: one divide per
                // sample instead of a transcendental, same endpoints and a
                // closed-form worst slope for the mip bias.
                let g = self.k;
                let bend = |x: f64| x / (x + (1.0 - x) * g);
                if p < 0.5 {
                    (0.5 * bend(2.0 * p), 1.0)
                } else {
                    (1.0 - 0.5 * bend(2.0 - 2.0 * p), 1.0)
                }
            }
            WarpMode::Mirror => {
                // Interpolates the identity toward the fold `1 - |2p - 1|`.
                let a = self.depth;
                if p < 0.5 {
                    (p * (1.0 + a), 1.0)
                } else {
                    (p + a * (2.0 - 3.0 * p), 1.0)
                }
            }
            WarpMode::Pwm => ((p / self.k).min(1.0), 1.0),
            WarpMode::Formant => {
                let q = p * self.k;
                // A parabolic window (no `cos`), faded in with the amount so
                // a small setting stays close to the dry wave.
                let window = 4.0 * p * (1.0 - p);
                let gain = 1.0 - self.depth * (1.0 - window);
                (q - q.floor(), gain as f32)
            }
            WarpMode::Quantize => {
                // Each step holds the table at its centre rather than its
                // start, so four steps of a sine read ±1 rather than 0.
                let n = self.k;
                (((p * n).floor() + 0.5) / n, 1.0)
            }
        }
    }
}

/// Split the polyBLEP correction for a step of height `h` between the
/// current sample and the next.
///
/// The step happens between this sample and the next one, `x` samples
/// (0..1) before the next. The two-sample polyBLEP residual — the
/// difference between a band-limited step (a linear-ramp integral of a
/// triangular kernel two samples wide) and the naive one — is `+h x²/2`
/// on the sample before the step and `-h (1-x)²/2` on the one after.
/// Returns `(add now, carry into the next sample)`.
///
/// The discontinuities this is used for — a sync reset, a warp's wrap
/// jump, a quantize step — are all predictable one sample ahead from the
/// phase and its increment, so the correction needs no lookahead delay
/// and adds no latency.
#[inline]
pub fn blep_split(h: f32, x: f32) -> (f32, f32) {
    let y = 1.0 - x;
    (0.5 * h * x * x, -0.5 * h * y * y)
}

/// The same split for a *slope* discontinuity — the polyBLAMP, the
/// integral of the polyBLEP residual: `d` is the change in slope, in output
/// units per sample, at the same point. `+d x³/6` before, `+d (1-x)³/6`
/// after.
#[inline]
pub fn blamp_split(d: f32, x: f32) -> (f32, f32) {
    let y = 1.0 - x;
    (d * x * x * x * (1.0 / 6.0), d * y * y * y * (1.0 / 6.0))
}
