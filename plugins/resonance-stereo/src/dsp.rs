//! The stereo width signal path (warmth-width-depth.md §6.2).
//!
//! Per sample, in this order:
//!
//! 1. **Widen** (`widen_mode`), restricted to the focus band:
//!    - *Decorrelate* — [`VelvetDecorrelator`]: pure side made from the
//!      mid, `(L + s, R − s)`. The mono sum is unchanged at any amount
//!      and any focus (up to float rounding). The default widener.
//!    - *Diffuse* — [`AllpassDecorrelator`]: per-side ERB all-pass
//!      cascades offset by `spread = amount · DIFFUSE_MAX_SPREAD` (0.25)
//!      of an ERB step. The mono fold's deepest notch grows with the
//!      spread (−0.3 dB at 0.1, −1.4 dB at 0.2, −2.3 dB at the 0.25 cap;
//!      `tests/stereo.rs`), so the cap keeps the whole amount range
//!      mono-safe. Uncapped, spread 0.5 notched −14.6 dB.
//!    - *Micro-shift* — two [`DopplerShifter`] voices made from the mid,
//!      +9 cents at 10 ms on the left and −9 cents at 15 ms on the
//!      right, added at `amount`. Moving combs in mono: where the two
//!      voices line up against the dry signal the fold dips by
//!      `20·log10(1 − amount)` (−4.4 dB at 0.4, −6 dB at 0.5, −14 dB at
//!      0.8), sweeping as the voices drift. Above amount 0.5 (a notch
//!      deeper than −6 dB) it is flagged as a mono risk like Haas.
//!    - *Haas* — the right channel above `focus_low` (an LR4 split; the
//!      band below stays in time) delayed by 1–30 ms and lowered 3 dB.
//!      The level offset keeps the mono comb's notches at
//!      `20·log10((1 − g) / 2)` ≈ −16.7 dB (`g` = −3 dB) instead of −∞
//!      — deeper, ≈ −21 to −23 dB, right around the exclude's LR4 corner,
//!      where the split's phase adds in — and the low exclude keeps the
//!      bass in time, but static combs remain: the mode is flagged as a
//!      mono risk ([`WidenMode::is_mono_risk`]).
//! 2. **Width** — M/S side gain ([`apply_width`]).
//! 3. **Mono-maker** — a high-pass on the side (6/12/24 dB/oct) at
//!    `mono_below`: side content below the corner is removed, which is
//!    the elliptical-EQ move of folding the bass into the mid. The mid is
//!    untouched, so the mono sum is too.
//! 4. **Balance**, then 5. **Rotation**.
//! 6. **Audition**: `solo_side` replaces the output with `(S, −S)`, then
//!    `mono_check` folds it to `(M, M)`. Both on is silence, which is the
//!    honest answer: side cancels in mono.
//!
//! # Parameter changes
//!
//! Width, balance, rotation, Micro-shift's level and Haas's delay ramp
//! per sample (20–50 ms). Decorrelate's amount ramps per sample too (its
//! velvet filter runs at unit amount and the ramp scales the side).
//! Diffuse's amount and focus are an all-pass layout, 80 coefficient sets
//! that cannot be ramped per sample at any sane cost, so a new layout is
//! crossfaded in over [`DIFFUSE_XFADE_MS`] instead (`Diffuser`).
//!
//! # Transparent defaults
//!
//! Every stage is skipped at its neutral value — width 1, mono-maker
//! off, widen off, balance 0, rotation 0, auditions off — so the default
//! plugin is a bit-exact passthrough.
//!
//! # Latency: none reported, by design
//!
//! Haas and Micro-shift add delay to the *wet* part only: Haas delays one
//! side against the other (the offset is the effect), and Micro-shift's
//! detuned voices sit 10–15 ms behind the dry signal, which stays in
//! time. Neither delays the signal as a whole, so the plugin reports 0
//! samples of latency in every mode, which is also constant (a plugin
//! cannot report a latency change, gap F2). They are deliberately offset
//! effects, not something PDC should undo.

use resonance_dsp::{
    apply_balance, apply_width, ms_decode, ms_encode, AllpassDecorrelator, Biquad, DelayLine,
    DopplerShifter, StereoRotation, VelvetDecorrelator,
};
use resonance_metering::CorrelationMeter;
use resonance_plugin::{Smoother, SmoothingStyle};

use crate::params::{StereoParams, FOCUS_HIGH_OPEN_HZ, MONO_BELOW_OFF_HZ};
use crate::viz::StereoViz;

/// Widening algorithms, in `widen_mode` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidenMode {
    Off,
    Decorrelate,
    Diffuse,
    MicroShift,
    Haas,
}

impl WidenMode {
    /// Choice labels, in range order. Haas carries its warning in its
    /// name, so a host's automation lane, the control API's param
    /// listing and the editor all say it.
    pub const LABELS: &'static [&'static str] = &[
        "Off",
        "Decorrelate",
        "Diffuse",
        "Micro-shift",
        "Haas (mono risk)",
    ];

    pub const ALL: [WidenMode; 5] = [
        WidenMode::Off,
        WidenMode::Decorrelate,
        WidenMode::Diffuse,
        WidenMode::MicroShift,
        WidenMode::Haas,
    ];

    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Self::Decorrelate,
            2 => Self::Diffuse,
            3 => Self::MicroShift,
            4 => Self::Haas,
            _ => Self::Off,
        }
    }

    pub fn index(self) -> i32 {
        self as i32
    }

    /// Whether the mode at `amount` puts deep combs into the mono fold.
    /// Haas always does (static combs). Micro-shift does above
    /// [`MICRO_SHIFT_RISK_AMOUNT`], where its moving combs' worst notch
    /// ([`micro_shift_notch_db`]) is deeper than −6 dB. Decorrelate keeps
    /// the mono sum exactly and Diffuse ripples by at most 2.3 dB.
    pub fn is_mono_risk(self, amount: f32) -> bool {
        match self {
            Self::Haas => true,
            Self::MicroShift => amount > MICRO_SHIFT_RISK_AMOUNT,
            _ => false,
        }
    }

    /// Whether the mono sum is left exactly as it was (up to rounding).
    pub fn preserves_mono_sum(self) -> bool {
        matches!(self, Self::Off | Self::Decorrelate)
    }

    /// One line on what `widen_amount` does in this mode, for the editor.
    pub fn amount_hint(self, amount: f32) -> String {
        match self {
            Self::Off => "Widening off".to_string(),
            Self::Decorrelate => format!(
                "Pure side from mid at {:.0}% — mono sum unchanged",
                amount * 100.0
            ),
            Self::Diffuse => format!(
                "All-pass spread {:.2} ERB — mono ripple under 2.5 dB",
                diffuse_spread(amount)
            ),
            Self::MicroShift => format!(
                "±{MICRO_CENTS:.0} cents voices at {:.0}% — {}moving combs in mono, down to {:.0} dB",
                amount * 100.0,
                if self.is_mono_risk(amount) { "MONO RISK: " } else { "" },
                micro_shift_notch_db(amount)
            ),
            Self::Haas => format!(
                "Right delayed {:.1} ms, {HAAS_LEVEL_DB:.0} dB — MONO RISK: combs in the fold",
                haas_delay_ms(amount)
            ),
        }
    }
}

/// Mono-maker slopes, in `mono_slope` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonoSlope {
    Db6,
    Db12,
    Db24,
}

impl MonoSlope {
    pub const LABELS: &'static [&'static str] = &["6 dB/oct", "12 dB/oct", "24 dB/oct"];

    pub fn from_index(i: i32) -> Self {
        match i {
            0 => Self::Db6,
            2 => Self::Db24,
            _ => Self::Db12,
        }
    }

    pub fn index(self) -> i32 {
        self as i32
    }
}

/// Micro-shift detune per voice, in cents (left up, right down).
pub const MICRO_CENTS: f32 = 9.0;
/// Micro-shift crossfade window.
pub const MICRO_WINDOW_MS: f32 = 20.0;
/// Base delays under the window, so the voices' mean delays are
/// 10 ms (left) and 15 ms (right).
pub const MICRO_BASE_L_MS: f32 = 0.0;
pub const MICRO_BASE_R_MS: f32 = 5.0;

/// Haas delay range, mapped from `widen_amount` 0..1.
pub const HAAS_MIN_MS: f32 = 1.0;
pub const HAAS_MAX_MS: f32 = 30.0;
/// Level of the delayed side.
pub const HAAS_LEVEL_DB: f32 = -3.0;
/// `focus_low` at or below this is "no low exclude" in Haas mode.
pub const HAAS_NO_EXCLUDE_HZ: f32 = 20.0;

/// Diffuse mode's all-pass sections per side.
pub const DIFFUSE_SECTIONS: usize = AllpassDecorrelator::DEFAULT_SECTIONS;
/// Diffuse mode's top edge when `focus_high` is open.
pub const DIFFUSE_OPEN_HIGH_HZ: f32 = AllpassDecorrelator::DEFAULT_HIGH_HZ;

/// Haas delay in ms for a `widen_amount`.
pub fn haas_delay_ms(amount: f32) -> f32 {
    HAAS_MIN_MS + (HAAS_MAX_MS - HAAS_MIN_MS) * amount.clamp(0.0, 1.0)
}

/// Largest Diffuse all-pass spread (fraction of an ERB step), reached at
/// `widen_amount` 1. Chosen by the mono fold's worst notch: −2.3 dB here,
/// against −14.6 dB at the all-pass cascade's own limit of 0.5.
pub const DIFFUSE_MAX_SPREAD: f32 = 0.25;

/// Diffuse all-pass spread (fraction of an ERB step) for a `widen_amount`.
pub fn diffuse_spread(amount: f32) -> f32 {
    DIFFUSE_MAX_SPREAD * amount.clamp(0.0, 1.0)
}

/// Micro-shift is a mono risk above this `widen_amount`: past it the
/// fold's worst notch, `20·log10(1 − amount)`, is deeper than −6 dB.
pub const MICRO_SHIFT_RISK_AMOUNT: f32 = 0.5;

/// The deepest dip Micro-shift's moving combs put into the mono fold at
/// a `widen_amount`, in dB: both voices in antiphase with the dry signal
/// leave `1 − amount` of it. Floored at −60 dB (a full null).
pub fn micro_shift_notch_db(amount: f32) -> f32 {
    20.0 * (1.0 - amount.clamp(0.0, 1.0)).max(1.0e-3).log10()
}

/// Samples between goniometer points pushed to the viz.
const GONIO_DECIMATION: u32 = 4;
/// Samples between correlation-strip pushes (≈ 21 ms at 48 kHz).
const CORRELATION_STEP: u32 = 1024;
/// Velvet seed: fixed, so renders are deterministic.
const VELVET_SEED: u64 = 0x5752_4544;

/// A linear parameter ramp settled on `v`. Retargeted once per block;
/// `Smoother::set_target` leaves a ramp in flight alone when the target
/// has not moved, so it lands exactly — which is what makes the neutral
/// values bit-exact.
fn ramp(sr: f32, ms: f32, v: f32) -> Smoother {
    let mut s = Smoother::new(SmoothingStyle::Linear(ms));
    s.set_sample_rate(sr);
    s.reset(v);
    s
}

/// Crossfade between two Diffuse all-pass layouts.
pub const DIFFUSE_XFADE_MS: f32 = 30.0;

/// Diffuse's all-pass cascade, re-laid-out without a step.
///
/// Re-laying out the cascade (a new spread or focus) swaps 80 sets of
/// coefficients at once, and ramping them per sample would mean
/// redesigning 80 biquads every sample. Instead a new layout goes into a
/// copy of the live cascade (same state, new coefficients) and the output
/// crossfades from the old cascade to the new one over
/// [`DIFFUSE_XFADE_MS`]; both run meanwhile. A layout asked for during a
/// fade waits for it to finish and then starts the next one, so a
/// continuous sweep follows in crossfaded steps of at most one fade.
struct Diffuser {
    live: AllpassDecorrelator,
    next: AllpassDecorrelator,
    /// Position of the fade into `next`; `None` when not fading.
    fade: Option<f32>,
    step: f32,
    /// The layout `live` has (or `next` is fading to), and the newest
    /// one asked for while a fade runs.
    layout: Option<(f32, f32, f32)>,
    pending: Option<(f32, f32, f32)>,
}

impl Diffuser {
    fn new(sr: f32) -> Self {
        Self {
            live: AllpassDecorrelator::default(),
            next: AllpassDecorrelator::default(),
            fade: None,
            step: 1.0 / (DIFFUSE_XFADE_MS * 0.001 * sr).max(1.0),
            layout: None,
            pending: None,
        }
    }

    fn configure(ap: &mut AllpassDecorrelator, sr: f32, (low, high, spread): (f32, f32, f32)) {
        ap.configure(sr, DIFFUSE_SECTIONS, low, high, spread);
    }

    /// Ask for a layout. The first one since construction or
    /// [`Self::reset`] applies at once; later ones crossfade.
    fn request(&mut self, sr: f32, layout: (f32, f32, f32)) {
        if self.layout.is_none() {
            Self::configure(&mut self.live, sr, layout);
            self.layout = Some(layout);
            return;
        }
        if self.fade.is_some() {
            self.pending = Some(layout);
            return;
        }
        if self.layout != Some(layout) {
            self.start_fade(sr, layout);
        }
    }

    fn start_fade(&mut self, sr: f32, layout: (f32, f32, f32)) {
        self.next = self.live;
        Self::configure(&mut self.next, sr, layout);
        self.layout = Some(layout);
        self.fade = Some(0.0);
        self.pending = None;
    }

    /// Forget the layout and the filter state: the next request applies
    /// at once, to a silent cascade.
    fn reset(&mut self) {
        self.live.reset();
        self.fade = None;
        self.pending = None;
        self.layout = None;
    }

    #[inline]
    fn process(&mut self, sr: f32, l: f32, r: f32) -> (f32, f32) {
        let Some(t) = self.fade else {
            return self.live.process(l, r);
        };
        let (al, ar) = self.live.process(l, r);
        let (bl, br) = self.next.process(l, r);
        let t = (t + self.step).min(1.0);
        let out = (al + t * (bl - al), ar + t * (br - ar));
        if t >= 1.0 {
            self.live = self.next;
            self.fade = None;
            if let Some(p) = self.pending.take() {
                if self.layout != Some(p) {
                    self.start_fade(sr, p);
                }
            }
        } else {
            self.fade = Some(t);
        }
        out
    }
}

/// Side-channel high-pass for the mono-maker at one of three slopes.
struct SideHighPass {
    slope: MonoSlope,
    freq: f32,
    /// 6 dB/oct: a one-pole low-pass subtracted from its input.
    lp1_coeff: f32,
    lp1_state: f32,
    /// 12 dB/oct: one Butterworth section; 24 dB/oct: two (LR4).
    bq: [Biquad; 2],
}

impl SideHighPass {
    fn new() -> Self {
        Self {
            slope: MonoSlope::Db12,
            freq: 0.0,
            lp1_coeff: 0.0,
            lp1_state: 0.0,
            bq: [Biquad::identity(); 2],
        }
    }

    fn configure(&mut self, sr: f32, slope: MonoSlope, freq: f32) {
        if slope == self.slope && freq == self.freq {
            return;
        }
        if slope != self.slope {
            self.reset();
        }
        self.slope = slope;
        self.freq = freq;
        let w = (std::f32::consts::TAU * freq / sr).min(std::f32::consts::PI);
        self.lp1_coeff = (-w).exp();
        let q = std::f32::consts::FRAC_1_SQRT_2;
        for b in &mut self.bq {
            b.set_high_pass(sr, freq, q);
        }
    }

    fn reset(&mut self) {
        self.lp1_state = 0.0;
        for b in &mut self.bq {
            b.reset();
        }
    }

    #[inline]
    fn process(&mut self, s: f32) -> f32 {
        match self.slope {
            MonoSlope::Db6 => {
                self.lp1_state = s + self.lp1_coeff * (self.lp1_state - s);
                s - self.lp1_state
            }
            MonoSlope::Db12 => self.bq[0].process(s),
            MonoSlope::Db24 => {
                let y = self.bq[0].process(s);
                self.bq[1].process(y)
            }
        }
    }
}

/// Band restriction for the Micro-shift voices' input.
struct Focus {
    hp: Biquad,
    lp: Biquad,
    lp_on: bool,
}

impl Focus {
    fn new() -> Self {
        Self {
            hp: Biquad::identity(),
            lp: Biquad::identity(),
            lp_on: false,
        }
    }

    fn configure(&mut self, sr: f32, low: f32, high: Option<f32>) {
        let q = std::f32::consts::FRAC_1_SQRT_2;
        self.hp.set_high_pass(sr, low, q);
        self.lp_on = high.is_some();
        if let Some(h) = high {
            self.lp.set_low_pass(sr, h, q);
        }
    }

    fn reset(&mut self) {
        self.hp.reset();
        self.lp.reset();
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.hp.process(x);
        if self.lp_on {
            self.lp.process(y)
        } else {
            y
        }
    }
}

pub struct StereoDsp {
    sr: f32,
    width: Smoother,
    balance: Smoother,
    rotation_deg: Smoother,
    rotation: StereoRotation,
    rotation_at: f32,

    mode: WidenMode,
    /// Last configured (focus_low, focus_high, amount) per mode, so the
    /// expensive re-layouts only run on a change.
    configured: Option<(f32, f32, f32)>,
    velvet: VelvetDecorrelator,
    /// Decorrelate's side amount, ramped per sample.
    decor_amount: Smoother,
    diffuse: Diffuser,
    micro_focus: Focus,
    micro_l: DopplerShifter,
    micro_r: DopplerShifter,
    micro_amount: Smoother,
    haas_line: DelayLine,
    /// LR4 split at `focus_low`: the low band stays in time.
    haas_low: [Biquad; 2],
    haas_high: [Biquad; 2],
    haas_exclude: bool,
    haas_delay: Smoother,
    haas_gain: f32,

    mono_on: bool,
    mono: SideHighPass,

    correlation: CorrelationMeter,
    gonio_count: u32,
    corr_count: u32,
}

impl StereoDsp {
    /// Allocates every delay line and filter for `sample_rate`; call from
    /// `initialize`. Smoothers start settled on the current params.
    pub fn new(sample_rate: f32, params: &StereoParams) -> Self {
        let sr = sample_rate.max(1.0);
        let smoother = |ms: f32, v: f32| ramp(sr, ms, v);
        let haas_max = (HAAS_MAX_MS * 0.001 * sr).ceil() as usize + 4;
        let mut micro_l = DopplerShifter::new(sr, MICRO_BASE_R_MS.max(MICRO_BASE_L_MS), MICRO_WINDOW_MS);
        let mut micro_r = DopplerShifter::new(sr, MICRO_BASE_R_MS.max(MICRO_BASE_L_MS), MICRO_WINDOW_MS);
        micro_l.set_base_delay(sr, MICRO_BASE_L_MS);
        micro_l.set_cents(MICRO_CENTS);
        micro_r.set_base_delay(sr, MICRO_BASE_R_MS);
        micro_r.set_cents(-MICRO_CENTS);
        let amount = params.widen_amount.value();
        Self {
            sr,
            width: smoother(20.0, params.width.value()),
            balance: smoother(20.0, params.balance.value()),
            rotation_deg: smoother(20.0, params.rotation.value()),
            rotation: StereoRotation::new(params.rotation.value().to_radians()),
            rotation_at: params.rotation.value(),
            mode: params.widen_mode(),
            configured: None,
            velvet: VelvetDecorrelator::new(sr, VELVET_SEED),
            decor_amount: smoother(20.0, amount),
            diffuse: Diffuser::new(sr),
            micro_focus: Focus::new(),
            micro_l,
            micro_r,
            micro_amount: smoother(20.0, amount),
            haas_line: DelayLine::new(haas_max),
            haas_low: [Biquad::identity(); 2],
            haas_high: [Biquad::identity(); 2],
            haas_exclude: false,
            haas_delay: smoother(50.0, haas_delay_ms(amount) * 0.001 * sr),
            haas_gain: resonance_dsp::db_to_linear(HAAS_LEVEL_DB),
            mono_on: false,
            mono: SideHighPass::new(),
            correlation: CorrelationMeter::new(sr),
            gonio_count: 0,
            corr_count: 0,
        }
    }

    fn reset_haas_split(&mut self) {
        for b in self.haas_low.iter_mut().chain(self.haas_high.iter_mut()) {
            b.reset();
        }
    }

    pub fn reset(&mut self) {
        self.velvet.reset();
        self.diffuse.reset();
        self.configured = None;
        self.micro_focus.reset();
        self.micro_l.reset();
        self.micro_r.reset();
        self.haas_line.clear();
        self.reset_haas_split();
        self.mono.reset();
        self.correlation.reset();
        self.gonio_count = 0;
        self.corr_count = 0;
    }

    /// Read the params once per block and reconfigure what changed.
    fn prepare_block(&mut self, p: &StereoParams) {
        self.width.set_target(p.width.value());
        self.balance.set_target(p.balance.value());
        self.rotation_deg.set_target(p.rotation.value());

        // Mono-maker.
        let mono_hz = p.mono_below.value();
        let was_on = self.mono_on;
        self.mono_on = mono_hz > MONO_BELOW_OFF_HZ;
        if self.mono_on {
            if !was_on {
                self.mono.reset();
            }
            self.mono
                .configure(self.sr, p.mono_slope(), mono_hz.min(0.45 * self.sr));
        }

        // Widening.
        let mode = p.widen_mode();
        if mode != self.mode {
            self.mode = mode;
            self.configured = None;
            // The incoming mode starts from silence rather than from
            // whatever it held the last time it ran.
            self.velvet.reset();
            self.diffuse.reset();
            // The incoming mode starts at its amount, not ramping from
            // wherever it was left.
            self.decor_amount.reset(p.widen_amount.value());
            self.micro_focus.reset();
            self.micro_l.reset();
            self.micro_r.reset();
            self.haas_line.clear();
            self.reset_haas_split();
        }
        let amount = p.widen_amount.value();
        let low = p.focus_low.value().min(0.45 * self.sr);
        let high_raw = p.focus_high.value();
        let high = (high_raw < FOCUS_HIGH_OPEN_HZ).then(|| high_raw.min(0.45 * self.sr));
        let key = (low, high.unwrap_or(0.0), amount);
        let changed = self.configured != Some(key);
        self.configured = Some(key);
        match mode {
            WidenMode::Off => {}
            WidenMode::Decorrelate => {
                if changed {
                    self.velvet.set_focus(self.sr, low, high.unwrap_or(0.0));
                }
                // The velvet filter runs at unit amount and the ramp
                // scales its side per sample; at a settled 0 it idles.
                self.decor_amount.set_target(amount);
                let active = amount > 0.0 || self.decor_amount.current() > 0.0;
                self.velvet.set_amount(if active { 1.0 } else { 0.0 });
            }
            WidenMode::Diffuse => {
                if changed {
                    let top = high.unwrap_or(DIFFUSE_OPEN_HIGH_HZ.min(0.45 * self.sr));
                    self.diffuse
                        .request(self.sr, (low, top.max(low + 1.0), diffuse_spread(amount)));
                }
            }
            WidenMode::MicroShift => {
                if changed {
                    self.micro_focus.configure(self.sr, low, high);
                }
                self.micro_amount.set_target(amount);
            }
            WidenMode::Haas => {
                if changed {
                    // At the bottom of `focus_low`'s range there is no
                    // exclude: the whole right side is delayed.
                    self.haas_exclude = p.focus_low.value() > HAAS_NO_EXCLUDE_HZ;
                    let q = std::f32::consts::FRAC_1_SQRT_2;
                    for b in &mut self.haas_low {
                        b.set_low_pass(self.sr, low, q);
                    }
                    for b in &mut self.haas_high {
                        b.set_high_pass(self.sr, low, q);
                    }
                }
                self.haas_delay.set_target(haas_delay_ms(amount) * 0.001 * self.sr);
            }
        }
    }

    #[inline]
    fn widen(&mut self, l: f32, r: f32) -> (f32, f32) {
        match self.mode {
            WidenMode::Off => (l, r),
            WidenMode::Decorrelate => {
                let a = self.decor_amount.next();
                let side = self.velvet.side(0.5 * (l + r));
                if a == 0.0 {
                    (l, r)
                } else {
                    let side = side * a;
                    (l + side, r - side)
                }
            }
            WidenMode::Diffuse => self.diffuse.process(self.sr, l, r),
            WidenMode::MicroShift => {
                let a = self.micro_amount.next();
                let m = self.micro_focus.process(0.5 * (l + r));
                let vl = self.micro_l.process(m);
                let vr = self.micro_r.process(m);
                (l + a * vl, r + a * vr)
            }
            WidenMode::Haas => {
                let (low, high) = if self.haas_exclude {
                    let lo = self.haas_low[0].process(r);
                    let lo = self.haas_low[1].process(lo);
                    let hi = self.haas_high[0].process(r);
                    let hi = self.haas_high[1].process(hi);
                    (lo, hi)
                } else {
                    (0.0, r)
                };
                self.haas_line.push(high);
                let d = self.haas_delay.next();
                // `tap(0)` is the sample just pushed, so `d` reads `d` back.
                let delayed = self.haas_line.tap_linear(d.max(0.0));
                (l, low + self.haas_gain * delayed)
            }
        }
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], p: &StereoParams, viz: &StereoViz) {
        self.prepare_block(p);
        let solo_side = p.solo_side.value();
        let mono_check = p.mono_check.value();
        let n = left.len().min(right.len());

        for i in 0..n {
            let (mut l, mut r) = self.widen(left[i], right[i]);

            let w = self.width.next();
            if w != 1.0 {
                (l, r) = apply_width(l, r, w);
            }

            if self.mono_on {
                let (m, s) = ms_encode(l, r);
                (l, r) = ms_decode(m, self.mono.process(s));
            }

            let b = self.balance.next();
            if b != 0.0 {
                (l, r) = apply_balance(l, r, b);
            }

            let deg = self.rotation_deg.next();
            if deg != self.rotation_at {
                self.rotation_at = deg;
                self.rotation.set_angle(deg.to_radians());
            }
            (l, r) = self.rotation.process(l, r);

            if solo_side {
                let s = 0.5 * (l - r);
                (l, r) = (s, -s);
            }
            if mono_check {
                let m = 0.5 * (l + r);
                (l, r) = (m, m);
            }

            left[i] = l;
            right[i] = r;

            self.gonio_count += 1;
            if self.gonio_count >= GONIO_DECIMATION {
                self.gonio_count = 0;
                viz.push_point(l, r);
            }
        }

        self.correlation.push_stereo(&left[..n], &right[..n]);
        self.corr_count += n as u32;
        if self.corr_count >= CORRELATION_STEP {
            self.corr_count %= CORRELATION_STEP;
            viz.push_correlation(self.correlation.correlation());
        }
        viz.store_block(self.correlation.correlation(), self.mode, p.widen_amount.value());
    }
}
