//! Character filter models: the non-clean alternatives to the linear
//! [`StateVariableFilter`](crate::dsp::filter::StateVariableFilter).
//!
//! Every model is a zero-delay-feedback (TPT) structure of one-pole
//! trapezoidal integrators with a single saturator in its resonance loop.
//! The loop is first solved *linearly* for the instantaneous feedback
//! signal, that estimate is pushed through the saturator, and the stages
//! are then run explicitly from the saturated value — the "cheap" ZDF
//! nonlinearity from Zavalishin, *The Art of VA Filter Design*. It keeps
//! the tuning of a proper ZDF filter (so cutoff stays put under fast
//! modulation and at 192 kHz) while the saturator bounds the loop, which is
//! what lets resonance go past the linear stability limit and
//! self-oscillate at a fixed, finite amplitude instead of blowing up.
//!
//! # Models and filter types
//!
//! The `filter_type` parameter (LP/HP/BP/Notch) is read per model:
//!
//! | model      | LP                  | HP                     | BP                     | Notch                  |
//! |------------|---------------------|------------------------|------------------------|------------------------|
//! | Clean SVF  | 12 dB (unchanged)   | 12 dB                  | 12 dB                  | 12 dB                  |
//! | Ladder 24  | 24 dB (4th tap)     | 24 dB (Xpander mix)    | 12+12 dB (tap mix)     | 2-pole tap mix         |
//! | Diode      | ~18–24 dB (4th tap) | → LP                   | → LP                   | → LP                   |
//! | MS-20      | 12 dB Korg35 LP     | 12 dB Korg35 HP        | band tap of the LP     | input − band tap       |
//! | NL SVF     | 12 dB               | 12 dB                  | 12 dB                  | 12 dB                  |
//!
//! The diode ladder is lowpass-only, as on the TB-303 it imitates: its
//! stages are coupled, so the tap mixing that turns a Moog ladder into an
//! Xpander does not produce clean HP/BP responses. Every other type falls
//! back to its lowpass ([`FilterModel::effective_type`]), and the editor
//! says so next to the type chips rather than silently ignoring the pick.
//!
//! # Resonance
//!
//! Resonance 0..1 maps linearly onto each model's loop gain
//! ([`FilterModel::feedback`]), with the top of the range set 20 % past
//! the model's linear self-oscillation threshold. The top ~15 % of the
//! knob therefore self-oscillates: a sine-like tone at the cutoff whose
//! amplitude the loop saturator pins (see [`SAT_HEADROOM`]). The clean SVF
//! is untouched — its resonance is still clamped to 0.99 and never
//! oscillates.

use crate::dsp::filter::FilterType;

/// Which filter circuit the voice filter emulates. `Clean` is the original
/// linear SVF and the default; it is rendered by `StateVariableFilter`, not
/// by [`CharacterFilter`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum FilterModel {
    Clean = 0,
    Ladder = 1,
    Diode = 2,
    Ms20 = 3,
    NonlinearSvf = 4,
}

/// Linear self-oscillation threshold of the 4-pole Moog ladder (loop gain
/// `k` at which the feedback reaches unity at cutoff).
const LADDER_K_CRIT: f32 = 4.0;
/// Linear self-oscillation threshold of the diode ladder with
/// [`DIODE_COUPLING`] = 0.5, measured from its analog transfer function.
const DIODE_K_CRIT: f32 = 8.61;
/// The Korg35 (MS-20) loop, `k·s/(1+s)²`, reaches unity at `k = 2`.
const MS20_K_CRIT: f32 = 2.0;
/// How far past the threshold full resonance goes. Far enough that the
/// oscillation starts reliably, settles quickly and comes out at a usable
/// level, not so far that the saturator is driven into a square wave.
const OVERDRIVE: f32 = 1.2;

/// Fraction of the ladder's resonance bass loss (`1/(1+k)`) that is made
/// up at the input. Half keeps the "resonance thins the bass" character
/// while stopping a fully resonant ladder from dropping ~15 dB.
const LADDER_BASS_COMP: f32 = 0.5;
/// The same for the diode ladder, kept lower: a 303 is supposed to lose
/// bottom end as the resonance comes up.
const DIODE_BASS_COMP: f32 = 0.3;

/// Coupling between adjacent diode-ladder stages: each stage also sees
/// `DIODE_COUPLING × (next stage − this stage)`. 0 would be a Moog ladder;
/// the coupling spreads the four poles, which is what gives the diode
/// ladder its softer slope and its squelchier, thinner resonance.
const DIODE_COUPLING: f32 = 0.5;
/// The coupled ladder resonates at 1.055× its stage cutoff; scaling the
/// integrator gain by the inverse puts the resonant peak exactly on the
/// cutoff knob. (Exact, not approximate: scaling `g = tan(πf/fs)` maps the
/// analog resonance at `1.055·ω` back onto `f` through the bilinear warp.)
const DIODE_TUNE: f32 = 1.0 / 1.055;

/// Extra damping the nonlinear SVF's resonance picks up as its band signal
/// saturates (see `nl_svf`). Sets how hard loud resonance is compressed and
/// where self-oscillation settles.
const NL_SVF_DAMP: f32 = 2.0;

/// Level at which the loop saturator starts to bite. A full-level single
/// oscillator (±1) passes a ladder with only mild colouring; drive and
/// resonance push it into the curve. It also sets the self-oscillation
/// amplitude, which lands in the same range as a full-level oscillator.
pub const SAT_HEADROOM: f32 = 1.5;
const INV_SAT_HEADROOM: f32 = 1.0 / SAT_HEADROOM;

impl FilterModel {
    /// Display names, indexed by the parameter's integer value.
    pub const LABELS: [&'static str; 5] = ["Clean SVF", "Ladder 24", "Diode", "MS-20", "NL SVF"];

    pub fn from_int(v: i32) -> Self {
        match v {
            1 => Self::Ladder,
            2 => Self::Diode,
            3 => Self::Ms20,
            4 => Self::NonlinearSvf,
            _ => Self::Clean,
        }
    }

    pub fn label(self) -> &'static str {
        Self::LABELS[self as usize]
    }

    /// The response a `filter_type` actually produces under this model —
    /// the diode ladder is lowpass-only (see the module docs).
    pub fn effective_type(self, filter_type: FilterType) -> FilterType {
        match self {
            Self::Diode => FilterType::Lowpass,
            _ => filter_type,
        }
    }

    /// The loop coefficient a 0..1 resonance maps to.
    ///
    /// For the ladders and the MS-20 this is the feedback gain `k`
    /// (self-oscillation past the model's `*_K_CRIT`). For the nonlinear SVF
    /// it is the SVF damping `k = 2 − 2.2·reso`, which crosses zero at
    /// ~91 % and goes slightly negative — i.e. undamped — above that. The
    /// clean SVF's own mapping lives in `StateVariableFilter::set_coeffs`.
    pub fn feedback(self, resonance: f32) -> f32 {
        let r = resonance.clamp(0.0, 1.0);
        match self {
            Self::Clean => 2.0 - 2.0 * r.min(0.99),
            Self::Ladder => r * LADDER_K_CRIT * OVERDRIVE,
            Self::Diode => r * DIODE_K_CRIT * OVERDRIVE,
            Self::Ms20 => r * MS20_K_CRIT * OVERDRIVE,
            Self::NonlinearSvf => 2.0 - 2.2 * r,
        }
    }
}

/// The loop saturator: `tanh` scaled to [`SAT_HEADROOM`].
///
/// [`resonance_dsp::tanh_fast`] — odd, monotonic and bounded to ±1 — so
/// every saturated signal here is bounded by `±SAT_HEADROOM`.
#[inline]
fn sat(x: f32) -> f32 {
    SAT_HEADROOM * resonance_dsp::tanh_fast(x * INV_SAT_HEADROOM)
}

/// One channel of a character filter. Holds up to four integrator states;
/// which ones are live depends on the model.
///
/// Same control-rate contract as the SVF: [`set_coeffs`](Self::set_coeffs)
/// does the `tan()` and the divides, [`process`](Self::process) is the
/// per-sample work. [`set_g`](Self::set_g) is the cheaper half of
/// `set_coeffs`, for the audio-rate filter-FM path.
#[derive(Clone)]
pub struct CharacterFilter {
    model: FilterModel,
    s: [f32; 4],

    /// Loop coefficient from [`FilterModel::feedback`].
    k: f32,
    /// Input gain: drive, times the model's bass compensation.
    in_gain: f32,

    /// Prewarped integrator gain `tan(π·fc/fs)` (diode: × [`DIODE_TUNE`]).
    g: f32,
    /// One-pole TPT gain `g/(1+g)` (ladder, MS-20).
    big_g: f32,
    /// Ladder: `G⁴`. MS-20: `1/(1 − k·G·(1−G))`.
    c0: f32,
    /// Nonlinear SVF coefficients, as in the clean SVF.
    a1: f32,
    a2: f32,
    /// Diode ladder: the Thomas-algorithm factors of its tridiagonal stage
    /// matrix (`cp` super-diagonal ratios, `inv_m` pivots), and `g·T⁻¹e₁`,
    /// the stages' response to the loop input.
    cp: [f32; 3],
    inv_m: [f32; 4],
    gc: [f32; 4],
}

impl CharacterFilter {
    pub fn new() -> Self {
        let mut f = Self {
            model: FilterModel::Ladder,
            s: [0.0; 4],
            k: 0.0,
            in_gain: 1.0,
            g: 0.0,
            big_g: 0.0,
            c0: 0.0,
            a1: 1.0,
            a2: 0.0,
            cp: [0.0; 3],
            inv_m: [1.0; 4],
            gc: [0.0; 4],
        };
        f.set_g(1.0);
        f
    }

    pub fn clear(&mut self) {
        self.s = [0.0; 4];
    }

    /// Precompute coefficients, at control rate. Same arguments and ranges
    /// as `StateVariableFilter::set_coeffs`, plus the model. `Clean` is
    /// accepted but rendered as the nonlinear SVF; the voice never routes
    /// the clean model here.
    pub fn set_coeffs(
        &mut self,
        model: FilterModel,
        cutoff_hz: f32,
        resonance: f32,
        sample_rate: f32,
        drive: f32,
    ) {
        self.model = model;
        let cutoff = cutoff_hz.clamp(20.0, sample_rate * 0.49);
        let g = (std::f32::consts::PI * cutoff / sample_rate).tan();
        self.k = model.feedback(resonance);
        let drive_gain = 1.0 + drive * 5.0;
        self.in_gain = drive_gain
            * match model {
                FilterModel::Ladder => 1.0 + LADDER_BASS_COMP * self.k,
                FilterModel::Diode => 1.0 + DIODE_BASS_COMP * self.k,
                _ => 1.0,
            };
        self.set_g(g);
    }

    /// Set the prewarped integrator gain `g = tan(π·fc/fs)` directly,
    /// keeping the model, resonance and drive from the last `set_coeffs`.
    /// This is the per-sample path for filter FM: no `tan()`, at most four
    /// divides (diode ladder), usually one.
    #[inline]
    pub fn set_g(&mut self, g: f32) {
        match self.model {
            FilterModel::Ladder => {
                let gg = g / (1.0 + g);
                self.g = g;
                self.big_g = gg;
                self.c0 = gg * gg * gg * gg;
            }
            FilterModel::Ms20 => {
                let gg = g / (1.0 + g);
                self.g = g;
                self.big_g = gg;
                // G(1−G) ≤ ¼ and k ≤ 2.4, so this stays ≥ 0.4.
                self.c0 = 1.0 / (1.0 - self.k * gg * (1.0 - gg));
            }
            FilterModel::Diode => {
                let g = g * DIODE_TUNE;
                self.g = g;
                // Rows: -g·y[i-1] + d[i]·y[i] + e·y[i+1] = rhs[i].
                let d = 1.0 + g * (1.0 + DIODE_COUPLING);
                let e = -g * DIODE_COUPLING;
                let inv_m0 = 1.0 / d;
                let cp0 = e * inv_m0;
                let inv_m1 = 1.0 / (d + g * cp0);
                let cp1 = e * inv_m1;
                let inv_m2 = 1.0 / (d + g * cp1);
                let cp2 = e * inv_m2;
                let inv_m3 = 1.0 / (1.0 + g + g * cp2);
                self.cp = [cp0, cp1, cp2];
                self.inv_m = [inv_m0, inv_m1, inv_m2, inv_m3];
                // T⁻¹·e₁, scaled by g.
                let q0 = inv_m0;
                let q1 = g * q0 * inv_m1;
                let q2 = g * q1 * inv_m2;
                let q3 = g * q2 * inv_m3;
                let c3 = q3;
                let c2 = q2 - cp2 * c3;
                let c1 = q1 - cp1 * c2;
                let c0 = q0 - cp0 * c1;
                self.gc = [g * c0, g * c1, g * c2, g * c3];
            }
            FilterModel::NonlinearSvf | FilterModel::Clean => {
                self.g = g;
                self.a1 = 1.0 / (1.0 + g * (g + self.k));
                self.a2 = g * self.a1;
            }
        }
    }

    /// Process one sample using the most recently set coefficients.
    #[inline]
    pub fn process(&mut self, input: f32, filter_type: FilterType) -> f32 {
        let x = input * self.in_gain;
        match self.model {
            FilterModel::Ladder => self.ladder(x, filter_type),
            FilterModel::Diode => self.diode(x),
            FilterModel::Ms20 => match filter_type {
                FilterType::Highpass => self.ms20_hp(x),
                _ => self.ms20_lp(x, filter_type),
            },
            FilterModel::NonlinearSvf | FilterModel::Clean => self.nl_svf(x, filter_type),
        }
    }

    /// Moog ladder: four identical one-poles, output fed back (inverted)
    /// through the input differential pair — the `sat` on `u`.
    #[inline]
    fn ladder(&mut self, x: f32, filter_type: FilterType) -> f32 {
        let (gg, k) = (self.big_g, self.k);
        let b = 1.0 - gg;
        let [s1, s2, s3, s4] = self.s;
        // y4 = G⁴·u + S, with S the states' contribution through the chain.
        let big_s = gg * (gg * (gg * b * s1 + b * s2) + b * s3) + b * s4;
        let u = sat((x - k * big_s) / (1.0 + k * self.c0));
        let y1 = gg * u + b * s1;
        let y2 = gg * y1 + b * s2;
        let y3 = gg * y2 + b * s3;
        let y4 = gg * y3 + b * s4;
        self.s = [2.0 * y1 - s1, 2.0 * y2 - s2, 2.0 * y3 - s3, 2.0 * y4 - s4];
        match filter_type {
            FilterType::Lowpass => y4,
            // (1−H)⁴: the Oberheim Xpander's 4-pole highpass mix.
            FilterType::Highpass => u - 4.0 * y1 + 6.0 * y2 - 4.0 * y3 + y4,
            // H²(1−H)², ×4 for unity gain at cutoff.
            FilterType::Bandpass => 4.0 * (y2 - 2.0 * y3 + y4),
            // (1−H)² + H² = 1 − 2H + 2H², zero at cutoff.
            FilterType::Notch => u - 2.0 * y1 + 2.0 * y2,
        }
    }

    /// Diode ladder: four coupled stages, solved as one tridiagonal system
    /// per sample with the factorisation cached by `set_g`.
    #[inline]
    fn diode(&mut self, x: f32) -> f32 {
        let (g, k) = (self.g, self.k);
        let [s1, s2, s3, s4] = self.s;
        let [cp0, cp1, cp2] = self.cp;
        let [m0, m1, m2, m3] = self.inv_m;
        // p = T⁻¹·s: the stage outputs with a zero loop input.
        let d0 = s1 * m0;
        let d1 = (s2 + g * d0) * m1;
        let d2 = (s3 + g * d1) * m2;
        let p4 = (s4 + g * d2) * m3;
        let p3 = d2 - cp2 * p4;
        let p2 = d1 - cp1 * p3;
        let p1 = d0 - cp0 * p2;
        // y = p + gc·u and u = x − k·y4, solved for u, then saturated.
        let u = sat((x - k * p4) / (1.0 + k * self.gc[3]));
        let y1 = p1 + self.gc[0] * u;
        let y2 = p2 + self.gc[1] * u;
        let y3 = p3 + self.gc[2] * u;
        let y4 = p4 + self.gc[3] * u;
        self.s = [2.0 * y1 - s1, 2.0 * y2 - s2, 2.0 * y3 - s3, 2.0 * y4 - s4];
        y4
    }

    /// Korg35 lowpass (MS-20 mk1): LP → LP, with the output's highpassed
    /// copy fed back *positively* into the second stage through the
    /// saturator. `s = [lp1, lp2, feedback-hp's lp, _]`.
    #[inline]
    fn ms20_lp(&mut self, x: f32, filter_type: FilterType) -> f32 {
        let (gg, k) = (self.big_g, self.k);
        let b = 1.0 - gg;
        let [s1, s2, s3, _] = self.s;
        let y1 = gg * x + b * s1;
        let y_lin = (gg * y1 - gg * k * b * s3 + b * s2) * self.c0;
        // The whole second-stage input clips, feedback and signal together —
        // the MS-20's diode limiter, and what makes it scream rather than
        // sing. It also bounds the output to ±SAT_HEADROOM.
        let u2 = sat(y1 + k * b * (y_lin - s3));
        let y = gg * u2 + b * s2;
        let lp3 = gg * y + b * s3;
        self.s = [2.0 * y1 - s1, 2.0 * y - s2, 2.0 * lp3 - s3, 0.0];
        // HP(LP(·)) — the loop's own band signal, ×2 for unity at cutoff.
        let band = 2.0 * (y - lp3);
        match filter_type {
            FilterType::Bandpass => band,
            FilterType::Notch => x - band,
            _ => y,
        }
    }

    /// Korg35 highpass (MS-20 mk1 HPF): HP → HP, with the output's
    /// lowpassed copy fed back into the second stage through the saturator.
    #[inline]
    fn ms20_hp(&mut self, x: f32) -> f32 {
        let (gg, k) = (self.big_g, self.k);
        let b = 1.0 - gg;
        let [s1, s2, s3, _] = self.s;
        let lp1 = gg * x + b * s1;
        let y1 = x - lp1;
        let y_lin = b * (y1 + k * b * s3 - s2) * self.c0;
        let u2 = sat(y1 + k * (gg * y_lin + b * s3));
        let lp2 = gg * u2 + b * s2;
        let y = u2 - lp2;
        let lp3 = gg * y + b * s3;
        self.s = [2.0 * lp1 - s1, 2.0 * lp2 - s2, 2.0 * lp3 - s3, 0.0];
        y
    }

    /// The Simper SVF with a saturating damping loop: the band signal's
    /// feedback runs through the saturator, so the more it clips, the more
    /// damping the filter sees. Loud resonance compresses instead of growing
    /// without bound, and with the base damping below zero (the top ~9 % of
    /// resonance) the filter oscillates at a level the saturator sets.
    ///
    /// Solved with the secant ("cheap Newton") method: a linear solve gives
    /// the band estimate, the saturator's secant gain at that estimate
    /// turns into an effective damping, and a second linear solve with that
    /// damping produces the sample. Putting the nonlinearity into the
    /// damping coefficient — inside the ZDF solve, scaled by `g` — keeps the
    /// oscillation level the same at every cutoff and sample rate, which a
    /// saturator applied to the raw integrator output does not.
    ///
    /// The input is saturated too (an OTA's input stage): with nothing else
    /// in the signal path clipping, drive would otherwise just be gain.
    /// `s = [ic1eq, ic2eq, _, _]`.
    #[inline]
    fn nl_svf(&mut self, x: f32, filter_type: FilterType) -> f32 {
        let x = sat(x);
        let [ic1, ic2, _, _] = self.s;
        let g = self.g;
        let v3 = x - ic2;
        let est = self.a1 * ic1 + self.a2 * v3;
        let secant = if est.abs() > 1e-6 { sat(est) / est } else { 1.0 };
        let k = self.k + NL_SVF_DAMP * (1.0 - secant);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let v1 = a1 * ic1 + g * a1 * v3;
        let v2 = ic2 + g * v1;
        self.s = [2.0 * v1 - ic1, 2.0 * v2 - ic2, 0.0, 0.0];
        match filter_type {
            FilterType::Lowpass => v2,
            FilterType::Highpass => x - k * v1 - v2,
            FilterType::Bandpass => v1,
            FilterType::Notch => x - k * v1,
        }
    }
}

impl Default for CharacterFilter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Audio-rate filter FM helpers
// ---------------------------------------------------------------------------

/// `tan(x)` for `0 ≤ x ≤ 0.49·π`, as a Padé [5/4] rational.
///
/// The filter-FM path needs a fresh `g = tan(π·fc/fs)` every sample, and a
/// libm `tan` there would cost more than the filter itself. The rational's
/// pole sits at 1.5707 — on top of π/2 — so it tracks `tan` to < 1e-6
/// relative through the audio band and to ~1 % at the 0.49·π clamp, which
/// is a frequency error of well under a cent.
#[inline]
pub fn tan_fast(x: f32) -> f32 {
    let x2 = x * x;
    x * (945.0 + x2 * (-105.0 + x2)) / (945.0 + x2 * (-420.0 + x2 * 15.0))
}

/// `2^x` for `|x| ≤ 16`: nearest integer through the exponent bits, the
/// ±½ remainder through a 5th-order Taylor series. Relative error < 3e-6
/// — inaudible as a cutoff error, and it keeps a libm `exp2` out of the
/// per-sample FM path.
#[inline]
pub fn exp2_fast(x: f32) -> f32 {
    let x = x.clamp(-16.0, 16.0);
    let xi = (x + 0.5).floor();
    let f = x - xi;
    let p = 1.0
        + f * (std::f32::consts::LN_2
            + f * (0.240_226_5 + f * (0.055_504_1 + f * (0.009_618_1 + f * 0.001_333_4))));
    p * f32::from_bits(((xi as i32 + 127) as u32) << 23)
}

// ---------------------------------------------------------------------------
// Analog magnitude response, for the editor's graph
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct C(f32, f32);

impl C {
    fn add(self, o: C) -> C {
        C(self.0 + o.0, self.1 + o.1)
    }
    fn sub(self, o: C) -> C {
        C(self.0 - o.0, self.1 - o.1)
    }
    fn mul(self, o: C) -> C {
        C(self.0 * o.0 - self.1 * o.1, self.0 * o.1 + self.1 * o.0)
    }
    fn scale(self, s: f32) -> C {
        C(self.0 * s, self.1 * s)
    }
    fn div(self, o: C) -> C {
        let d = (o.0 * o.0 + o.1 * o.1).max(1e-20);
        C(
            (self.0 * o.0 + self.1 * o.1) / d,
            (self.1 * o.0 - self.0 * o.1) / d,
        )
    }
    fn norm_sq(self) -> f32 {
        self.0 * self.0 + self.1 * self.1
    }
}

const ONE: C = C(1.0, 0.0);

/// Small-signal magnitude of a character model at `freq`, in dB, from the
/// model's analog prototype (saturators taken as unity gain). Used by the
/// editor's response graph; the clean SVF keeps its own curve there.
///
/// Past the self-oscillation threshold the linear loop has no steady-state
/// response; the peak then just reads as very large, which the graph clamps.
pub fn response_db(
    model: FilterModel,
    filter_type: FilterType,
    freq: f32,
    cutoff: f32,
    resonance: f32,
) -> f32 {
    let w = (freq / cutoff.max(1.0)).max(1e-6);
    let s = C(0.0, w);
    let k = model.feedback(resonance);
    // One-pole lowpass / highpass at the cutoff.
    let lp = ONE.div(ONE.add(s));
    let hp = s.div(ONE.add(s));
    let ft = model.effective_type(filter_type);

    let h = match model {
        FilterModel::Ladder => {
            let lp2 = lp.mul(lp);
            let lp4 = lp2.mul(lp2);
            let u = C(1.0 + LADDER_BASS_COMP * k, 0.0).div(ONE.add(lp4.scale(k)));
            let lp3 = lp2.mul(lp);
            match ft {
                FilterType::Lowpass => u.mul(lp4),
                FilterType::Highpass => {
                    let one_m = ONE.sub(lp);
                    let hp2 = one_m.mul(one_m);
                    u.mul(hp2.mul(hp2))
                }
                FilterType::Bandpass => u.mul(lp2.sub(lp3.scale(2.0)).add(lp4)).scale(4.0),
                FilterType::Notch => u.mul(ONE.sub(lp.scale(2.0)).add(lp2.scale(2.0))),
            }
        }
        FilterModel::Diode => {
            // (s'·I − A)·y = e₁ with s' = s / DIODE_TUNE, by Thomas.
            let sd = s.scale(1.0 / DIODE_TUNE);
            let c = DIODE_COUPLING;
            let d = sd.add(C(1.0 + c, 0.0));
            let d_last = sd.add(ONE);
            let e = C(-c, 0.0);
            let a = C(-1.0, 0.0);
            let cp0 = e.div(d);
            let m1 = d.sub(a.mul(cp0));
            let cp1 = e.div(m1);
            let m2 = d.sub(a.mul(cp1));
            let cp2 = e.div(m2);
            let m3 = d_last.sub(a.mul(cp2));
            let q0 = ONE.div(d);
            let q1 = C(0.0, 0.0).sub(a.mul(q0)).div(m1);
            let q2 = C(0.0, 0.0).sub(a.mul(q1)).div(m2);
            let h4 = C(0.0, 0.0).sub(a.mul(q2)).div(m3);
            C(1.0 + DIODE_BASS_COMP * k, 0.0).mul(h4).div(ONE.add(h4.scale(k)))
        }
        FilterModel::Ms20 => {
            let loop_den = ONE.sub(lp.mul(hp).scale(k));
            match ft {
                FilterType::Highpass => hp.mul(hp).div(loop_den),
                _ => {
                    let y = lp.mul(lp).div(loop_den);
                    let band = y.mul(hp).scale(2.0);
                    match ft {
                        FilterType::Bandpass => band,
                        FilterType::Notch => ONE.sub(band),
                        _ => y,
                    }
                }
            }
        }
        FilterModel::NonlinearSvf | FilterModel::Clean => {
            // Below zero the damping has no steady state; draw the peak.
            let damp = k.max(0.02);
            let den = s.mul(s).add(s.scale(damp)).add(ONE);
            match ft {
                FilterType::Lowpass => ONE.div(den),
                FilterType::Highpass => s.mul(s).div(den),
                FilterType::Bandpass => s.div(den),
                FilterType::Notch => s.mul(s).add(ONE).div(den),
            }
        }
    };
    10.0 * h.norm_sq().max(1e-10).log10()
}
