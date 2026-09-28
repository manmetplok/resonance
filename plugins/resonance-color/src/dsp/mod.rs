//! The character DSP (warmth-width-depth.md §6.1; Tape's HQ quality is
//! slice W6b).
//!
//! # Signal flow, per channel
//!
//! ```text
//!             ┌──────────── oversampler (stage rate) ────────────┐
//!  x ──┬──► up ─► response pre ─► curve (ADAA) ─► response post ─► DC ─► down ─► mode post ─► tone ─► ×auto ─┐
//!      │                                                                                                    mix ─► ×output ─► flutter (Tape)
//!      └──► up ─────────────────────── (same filters, nothing else) ──────────────────────► down ─── dry ────┘
//! ```
//!
//! - **The curve** is a W5 [`Curve`] with first-order ADAA ([`Adaa1`]),
//!   run inside the latency-free IIR [`Oversampler`] at the `oversample`
//!   factor (Off / 2× / 4×). Transformer mode uses the
//!   [`LfWeightedDrive`] stage instead, which wraps its own ADAA curve in
//!   an LF emphasis pair, a sub-sonic high-pass and an HF resonance.
//! - **Tape HQ** (`tape_quality` = HQ) swaps only the curve for a
//!   Jiles-Atherton [`Hysteresis`] stage per channel (voicing in
//!   [`voicing::tape_hq`]), integrated by the `tape_solver` choice (RK4 by
//!   default). The hysteresis is stateful, so ADAA does not apply: the
//!   stage runs at [`Settings::stage_factor`], the `oversample` factor
//!   raised to at least 2×, through the same latency-free IIR pair (the
//!   dry path follows it, so `mix` stays phase-aligned). The head bump,
//!   HF loss, flutter, tone and auto-gain are shared with Standard.
//!   Entering HQ starts the loop demagnetised.
//! - **Mode post** is Tape's [`HeadBump`] and level-dependent [`HfLoss`],
//!   at the base rate after the downsampler.
//! - **Dry** runs through a second oversampler pair of the same design
//!   and factor, so dry and wet share the up/down filters' phase response
//!   exactly and `mix` never comb-filters. At `Off` both pairs are exact
//!   passthroughs.
//!
//! # ADAA at 1×, and why the dry path is not averaged to match
//!
//! On the linear part of a curve, first-order ADAA is the two-tap average
//! `(1 + z⁻¹)/2` (W5: −3 dB at fs/4, a null at Nyquist). At 2× and 4×
//! that average sits at the stage rate, above the audio band's top
//! octave, which is one reason the default is 2×. At `Off` the wet path
//! carries it in-band. The dry path is deliberately **not** run through a
//! matching average: the difference between the two is a one-sample FIR,
//! so the blend `(1−m) + m·(1+z⁻¹)/2` has magnitude
//! `√((1−m)² + (2m−m²)·cos²(ω/2))`, which falls monotonically from DC to
//! Nyquist for every `m` — there is no notch anywhere in the band, only
//! the wet signal's own top-octave softening scaled by `mix`. Averaging
//! the dry path would instead soften the top octave at every mix,
//! including 0, and cost `mix = 0` its bit-exact passthrough at `Off`.
//!
//! # Exact paths
//!
//! - `mix = 0` returns the dry path exactly: the input itself at `Off`,
//!   the IIR round trip at 2× / 4× (see `tests/transparency.rs`).
//! - Console at `drive = 0` with `response`, `tone` and `flutter` at 0 is
//!   the identity curve, which ADAA passes straight through, so its wet
//!   path equals its dry path sample for sample; auto-gain then computes
//!   a gain of exactly 1.
//!
//! # Flutter
//!
//! Switching [`Flutter`] on inserts its ≈ 1.1 ms centre delay (W5), so the
//! stage never switches: turning flutter on or off (or leaving Tape mode)
//! crossfades between the undelayed and the flutter-delayed signal over
//! [`FLUTTER_FADE_MS`]. At 0 it is bypassed outright (ToTape6-style: no
//! delay, no interpolation). Flutter moves the whole output, like a tape
//! transport: applying it to the wet path alone would make `mix` a
//! chorus.
//!
//! # RT safety
//!
//! [`ColorDsp::new`] allocates (the flutter delay lines) and designs every
//! filter; call it from `initialize`. [`ColorDsp::process`] allocates
//! nothing and takes no locks.

pub mod voicing;

use resonance_dsp::tape::hf_loss_corner_hz;
use resonance_dsp::{
    db_to_linear, linear_to_db, Adaa1, Biquad, Curve, DcBlocker, EmphasisPair, Flutter,
    HeadBump, HfLoss, Hysteresis, HysteresisSolver, JaParams, LfWeightedDrive, OversampleFactor,
    Oversampler,
};
use resonance_metering::k_weighting::KWeightingFilter;
use resonance_plugin::{Smoother, SmoothingStyle};

use crate::params::{speed_ips, ColorParams, Mode, TapeQuality};
use crate::viz::ColorViz;

/// Crossfade length when flutter's centre delay comes or goes.
pub const FLUTTER_FADE_MS: f32 = 30.0;

/// Time constant of the auto-gain power followers. Long enough that the
/// gain does not ride individual hits; both followers share it, so their
/// ratio is right from the first sample (both rise from zero together).
pub const AUTO_GAIN_TIME_S: f32 = 0.8;
/// Auto-gain never moves the wet signal further than this, either way.
pub const AUTO_GAIN_LIMIT_DB: f32 = 24.0;
/// Floor added to both follower powers (−100 dBFS²): in silence the
/// ratio drifts to 1 instead of dividing two denormals.
const AUTO_GAIN_EPS: f64 = 1.0e-10;

/// Parameter de-zipper times.
const SMOOTH_MS: f32 = 20.0;
const AUTO_GAIN_BLEND_MS: f32 = 50.0;
const FLUTTER_SMOOTH_MS: f32 = 50.0;

/// Largest input magnitude the chain accepts (≈ +80 dBFS); beyond it the
/// sample is clamped, and non-finite samples read as 0.
const INPUT_LIMIT: f32 = 1.0e4;

#[inline]
fn sanitize(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(-INPUT_LIMIT, INPUT_LIMIT)
    } else {
        0.0
    }
}

/// One block's parameter snapshot. Read from the params once per block
/// by the plugin, or built directly by the probe and the tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub mode: Mode,
    pub drive: f32,
    pub bias: f32,
    pub response_db: f32,
    pub tone_db: f32,
    pub mix: f32,
    pub auto_gain: bool,
    pub output_db: f32,
    pub oversample: OversampleFactor,
    pub speed_ips: f32,
    pub flutter: f32,
    pub tape_quality: TapeQuality,
    pub tape_solver: HysteresisSolver,
}

impl Settings {
    /// Tape mode at HQ: the hysteresis stage replaces the curve.
    pub fn is_hq(&self) -> bool {
        self.mode == Mode::Tape && self.tape_quality == TapeQuality::Hq
    }

    /// The oversampling the stage actually runs at: the `oversample`
    /// setting, raised to at least 2x in Tape HQ (hysteresis is stateful,
    /// so ADAA cannot stand in for oversampling there). Still the IIR
    /// pair, so still latency-free.
    pub fn stage_factor(&self) -> OversampleFactor {
        if self.is_hq() && self.oversample == OversampleFactor::Off {
            OversampleFactor::X2
        } else {
            self.oversample
        }
    }

    pub fn from_params(p: &ColorParams) -> Self {
        Self {
            mode: p.mode(),
            drive: p.drive.value(),
            bias: p.bias.value(),
            response_db: p.response.value(),
            tone_db: p.tone.value(),
            mix: p.mix.value(),
            auto_gain: p.auto_gain.value(),
            output_db: p.output.value(),
            oversample: p.oversample_factor(),
            speed_ips: speed_ips(p.speed.value()),
            flutter: p.flutter.value(),
            tape_quality: p.tape_quality(),
            tape_solver: p.tape_solver(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_params(&ColorParams::default())
    }
}

/// Per-sample curve state shared by both channels and every sub-sample.
#[derive(Clone, Copy)]
struct Voice {
    curve: Curve,
    /// Curve input gain (unused in Console, whose drive is in the curve).
    gain: f32,
    /// `1 / (gain · f'(0))` for the gain-driven modes, `1 / f'(0)` for
    /// Transformer (whose stage already divides by the gain), 1 in Console,
    /// `1 / (dM_an/dH)` in Tape HQ.
    norm: f32,
    /// The hysteresis constants (Tape HQ only; the default otherwise).
    ja: JaParams,
}

struct Channel {
    os_wet: Oversampler,
    os_dry: Oversampler,
    response: EmphasisPair,
    adaa: Adaa1,
    xfmr: LfWeightedDrive,
    dc: DcBlocker,
    bump: HeadBump,
    hf: HfLoss,
    tone_lo: Biquad,
    tone_hi: Biquad,
    flutter: Flutter,
    hyst: Hysteresis,
}

impl Channel {
    fn new(sample_rate: f32) -> Self {
        Self {
            os_wet: Oversampler::new(),
            os_dry: Oversampler::new(),
            response: EmphasisPair::new(),
            adaa: Adaa1::new(),
            xfmr: LfWeightedDrive::new(),
            dc: DcBlocker::default(),
            bump: HeadBump::new(),
            hf: HfLoss::new(),
            tone_lo: Biquad::identity(),
            tone_hi: Biquad::identity(),
            flutter: Flutter::new(sample_rate),
            hyst: Hysteresis::default(),
        }
    }

    fn reset(&mut self) {
        self.os_wet.reset();
        self.os_dry.reset();
        self.reset_stage();
        self.bump.reset();
        self.hf.reset();
        self.tone_lo.reset();
        self.tone_hi.reset();
        self.flutter.reset();
    }

    /// State that belongs to the stage rate, cleared when it changes.
    fn reset_stage(&mut self) {
        self.response.reset();
        self.adaa.reset();
        self.xfmr.reset();
        self.dc.reset();
        self.hyst.reset();
    }

    /// One base-rate sample: `(dry, wet)` before auto-gain and mix.
    #[inline]
    fn tick(&mut self, x: f32, v: &Voice, mode: Mode, flags: &Flags) -> (f32, f32) {
        let ratio = self.os_wet.ratio();

        let dry_buf = self.os_dry.upsample(x);
        let dry = self.os_dry.downsample(&dry_buf);

        let mut buf = self.os_wet.upsample(x);
        for s in &mut buf[..ratio] {
            let e = self.response.pre(*s);
            let y = match mode {
                Mode::Console => self.adaa.process(&v.curve, e),
                Mode::Transformer => {
                    self.xfmr.set_curve(v.curve);
                    self.xfmr.set_drive(v.gain);
                    self.xfmr.process(e) * v.norm
                }
                Mode::Tape if flags.hq => {
                    if self.hyst.params() != v.ja {
                        self.hyst.set_params(v.ja);
                    }
                    self.hyst.process(e as f64) as f32 * v.norm
                }
                _ => self.adaa.process(&v.curve, v.gain * e) * v.norm,
            };
            let y = self.response.post(y);
            *s = if flags.dc_block { self.dc.process(y) } else { y };
        }
        let mut wet = self.os_wet.downsample(&buf);

        if mode == Mode::Tape {
            wet = self.hf.process(self.bump.process(wet));
        }
        if flags.tone_on {
            wet = self.tone_hi.process(self.tone_lo.process(wet));
        }
        (dry, wet)
    }
}

/// Block-constant switches derived from the settings.
#[derive(Clone, Copy, Default)]
struct Flags {
    dc_block: bool,
    tone_on: bool,
    /// Tape HQ: the hysteresis stage replaces the curve.
    hq: bool,
}

/// Stereo-linked, K-weighted RMS match of the wet path to the dry one.
struct AutoGain {
    k_dry: [KWeightingFilter; 2],
    k_wet: [KWeightingFilter; 2],
    p_dry: f64,
    p_wet: f64,
    coef: f64,
    limit: f64,
}

impl AutoGain {
    fn new(sample_rate: f32) -> Self {
        Self {
            k_dry: [KWeightingFilter::new(sample_rate); 2],
            k_wet: [KWeightingFilter::new(sample_rate); 2],
            p_dry: 0.0,
            p_wet: 0.0,
            coef: (-1.0 / (AUTO_GAIN_TIME_S as f64 * sample_rate.max(1.0) as f64)).exp(),
            limit: db_to_linear(AUTO_GAIN_LIMIT_DB) as f64,
        }
    }

    fn reset(&mut self) {
        for k in self.k_dry.iter_mut().chain(self.k_wet.iter_mut()) {
            k.reset();
        }
        self.p_dry = 0.0;
        self.p_wet = 0.0;
    }

    /// Feed one frame; returns the gain that matches wet to dry.
    #[inline]
    fn gain(&mut self, dry: [f32; 2], wet: [f32; 2]) -> f32 {
        let mut pd = 0.0f64;
        let mut pw = 0.0f64;
        for c in 0..2 {
            let d = self.k_dry[c].process(dry[c]) as f64;
            let w = self.k_wet[c].process(wet[c]) as f64;
            pd += d * d;
            pw += w * w;
        }
        let a = 1.0 - self.coef;
        self.p_dry += (pd - self.p_dry) * a;
        self.p_wet += (pw - self.p_wet) * a;
        let g = ((self.p_dry + AUTO_GAIN_EPS) / (self.p_wet + AUTO_GAIN_EPS)).sqrt();
        if g.is_finite() {
            g.clamp(1.0 / self.limit, self.limit) as f32
        } else {
            1.0
        }
    }
}

pub struct ColorDsp {
    sample_rate: f32,
    /// The settings the derived state was last built for; `None` before
    /// the first block, which forces a full configure.
    configured: Option<Settings>,
    ch: [Channel; 2],
    auto: AutoGain,
    flags: Flags,

    drive_s: Smoother,
    bias_s: Smoother,
    mix_s: Smoother,
    output_s: Smoother,
    auto_s: Smoother,
    response_s: Smoother,
    tone_s: Smoother,
    flutter_s: Smoother,

    /// Response and tone as last applied to the filter coefficients.
    response_applied: f32,
    tone_applied: f32,

    /// Curve state cache: recomputed only when drive or bias moved.
    voice: Voice,
    voice_key: (f32, f32, Mode, bool),

    /// Flutter crossfade position (0 = bypassed, 1 = fully fluttered)
    /// and step per sample; the last non-zero amount, held while fading
    /// out so the delay does not snap away.
    flutter_fade: f32,
    flutter_step: f32,
    flutter_held: f32,

    /// Decaying peak meters (linear) and the last applied auto-gain.
    in_peak: f32,
    out_peak: f32,
    meter_decay: f32,
    last_auto_gain: f32,
}

impl ColorDsp {
    /// Allocates the flutter delay lines and designs every filter. Call
    /// from `initialize`, never from the audio callback.
    pub fn new(sample_rate: f32, settings: &Settings) -> Self {
        let sr = sample_rate.max(1.0);
        let smoother = |style| {
            let mut s = Smoother::new(style);
            s.set_sample_rate(sr);
            s
        };
        let mut dsp = Self {
            sample_rate: sr,
            configured: None,
            ch: [Channel::new(sr), Channel::new(sr)],
            auto: AutoGain::new(sr),
            flags: Flags::default(),
            drive_s: smoother(SmoothingStyle::Linear(SMOOTH_MS)),
            bias_s: smoother(SmoothingStyle::Linear(SMOOTH_MS)),
            mix_s: smoother(SmoothingStyle::Linear(SMOOTH_MS)),
            output_s: smoother(SmoothingStyle::Linear(SMOOTH_MS)),
            auto_s: smoother(SmoothingStyle::Linear(AUTO_GAIN_BLEND_MS)),
            response_s: smoother(SmoothingStyle::Linear(SMOOTH_MS)),
            tone_s: smoother(SmoothingStyle::Linear(SMOOTH_MS)),
            flutter_s: smoother(SmoothingStyle::Linear(FLUTTER_SMOOTH_MS)),
            response_applied: f32::NAN,
            tone_applied: f32::NAN,
            voice: Voice {
                curve: Curve::Tanh,
                gain: 1.0,
                norm: 1.0,
                ja: JaParams::default(),
            },
            voice_key: (f32::NAN, f32::NAN, Mode::Tube, false),
            flutter_fade: 0.0,
            flutter_step: 1.0 / (FLUTTER_FADE_MS * 0.001 * sr).max(1.0),
            flutter_held: 0.0,
            in_peak: 0.0,
            out_peak: 0.0,
            meter_decay: (-1.0 / (0.25 * sr)).exp(),
            last_auto_gain: 1.0,
        };
        dsp.seed(settings);
        dsp
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Land every smoother on `s` without ramping, so a fresh instance
    /// (or a reset one) starts where its parameters are.
    fn seed(&mut self, s: &Settings) {
        self.drive_s.reset(s.drive);
        self.bias_s.reset(s.bias);
        self.mix_s.reset(s.mix.clamp(0.0, 1.0));
        self.output_s.reset(s.output_db);
        self.auto_s.reset(if s.auto_gain { 1.0 } else { 0.0 });
        self.response_s.reset(s.response_db);
        self.tone_s.reset(s.tone_db);
        let fl = s.flutter.clamp(0.0, 1.0);
        self.flutter_held = fl;
        self.flutter_s.reset(fl);
        self.flutter_fade = if s.mode == Mode::Tape && fl > 0.0 { 1.0 } else { 0.0 };
        self.configure(s);
    }

    /// Clear every piece of signal state (filters, followers, flutter
    /// history) and snap the smoothers to `s`.
    pub fn reset(&mut self, s: &Settings) {
        for ch in &mut self.ch {
            ch.reset();
        }
        self.auto.reset();
        self.in_peak = 0.0;
        self.out_peak = 0.0;
        self.last_auto_gain = 1.0;
        self.configured = None;
        self.seed(s);
    }

    fn stage_rate(&self, factor: OversampleFactor) -> f32 {
        self.sample_rate * factor.ratio() as f32
    }

    /// Rebuild whatever the settings changed. Cheap when nothing did.
    fn configure(&mut self, s: &Settings) {
        let prev = self.configured;
        let factor_changed = prev.is_none_or(|p| p.stage_factor() != s.stage_factor());
        let speed_changed = prev.is_none_or(|p| p.speed_ips != s.speed_ips);

        if factor_changed {
            let stage = self.stage_rate(s.stage_factor());
            for ch in &mut self.ch {
                // A factor change clears the oversamplers (their state
                // belongs to the old rate); so must everything that ran
                // at that rate.
                ch.os_wet.set_factor(s.stage_factor());
                ch.os_dry.set_factor(s.stage_factor());
                ch.reset_stage();
                ch.dc.set_cutoff(DcBlocker::DEFAULT_CUTOFF_HZ, stage);
                ch.xfmr.set_emphasis(
                    stage,
                    voicing::TRANSFORMER_LF_CORNER_HZ,
                    voicing::TRANSFORMER_LF_BOOST_DB,
                );
                ch.xfmr.set_subsonic(
                    stage,
                    voicing::TRANSFORMER_SUBSONIC_HZ,
                    LfWeightedDrive::SUBSONIC_Q,
                );
                ch.xfmr.set_hf_resonance(
                    stage,
                    voicing::TRANSFORMER_HF_RES_HZ.min(0.45 * stage),
                    voicing::TRANSFORMER_HF_RES_DB,
                );
            }
            // The emphasis pair lives at the stage rate too.
            self.response_applied = f32::NAN;
        }

        if speed_changed {
            let sr = self.sample_rate;
            for ch in &mut self.ch {
                ch.bump
                    .set(sr, s.speed_ips, voicing::TAPE_BUMP_DB, voicing::TAPE_DIP_DB);
                ch.hf.set_corner(sr, hf_loss_corner_hz(s.speed_ips));
                ch.hf.set_times(sr, voicing::TAPE_HF_ATTACK_MS, voicing::TAPE_HF_RELEASE_MS);
                ch.hf.set_amounts(
                    voicing::TAPE_HF_STATIC_DB,
                    voicing::TAPE_HF_DYNAMIC_DB,
                    voicing::TAPE_HF_REFERENCE,
                );
            }
        }

        // Entering Tape HQ starts the hysteresis from the demagnetised
        // state: whatever it held from an earlier HQ stretch belongs to
        // signal that is long gone.
        let hq_entered = s.is_hq() && prev.is_none_or(|p| !p.is_hq());
        let solver_changed = prev.is_none_or(|p| p.tape_solver != s.tape_solver);
        if hq_entered || solver_changed {
            for ch in &mut self.ch {
                if hq_entered {
                    ch.hyst.reset();
                }
                ch.hyst.set_solver(s.tape_solver);
            }
        }

        self.flags.dc_block = voicing::needs_dc_block(s.mode);
        self.flags.hq = s.is_hq();
        self.configured = Some(*s);
        self.apply_filters(s.stage_factor());
    }

    /// Push the smoothed response and tone into the filter coefficients
    /// when they moved.
    fn apply_filters(&mut self, factor: OversampleFactor) {
        let response = self.response_s.current();
        if response.to_bits() != self.response_applied.to_bits() {
            let stage = self.stage_rate(factor);
            for ch in &mut self.ch {
                ch.response.set(stage, voicing::RESPONSE_CORNER_HZ, response);
            }
            self.response_applied = response;
        }
        let tone = self.tone_s.current();
        if tone.to_bits() != self.tone_applied.to_bits() {
            let sr = self.sample_rate;
            for ch in &mut self.ch {
                ch.tone_lo
                    .set_low_shelf(sr, voicing::TONE_PIVOT_HZ, voicing::TONE_Q, -0.5 * tone);
                ch.tone_hi
                    .set_high_shelf(sr, voicing::TONE_PIVOT_HZ, voicing::TONE_Q, 0.5 * tone);
            }
            self.tone_applied = tone;
            self.flags.tone_on = tone != 0.0;
        }
    }

    #[inline]
    fn voice_for(&mut self, mode: Mode, hq: bool, drive: f32, bias: f32) -> Voice {
        let key = (drive, bias, mode, hq);
        if key.0.to_bits() == self.voice_key.0.to_bits()
            && key.1.to_bits() == self.voice_key.1.to_bits()
            && key.2 == self.voice_key.2
            && key.3 == self.voice_key.3
        {
            return self.voice;
        }
        let amount = voicing::drive_amount(mode, drive);
        if hq {
            let (ja, norm) = voicing::tape_hq(amount, bias);
            self.voice = Voice {
                curve: Curve::Tanh,
                gain: amount,
                norm,
                ja,
            };
            self.voice_key = key;
            return self.voice;
        }
        let curve = voicing::curve(mode, amount, bias);
        let slope = curve.slope_at_zero() as f32;
        let norm = match mode {
            Mode::Console => 1.0,
            Mode::Transformer => 1.0 / slope,
            _ => 1.0 / (amount * slope),
        };
        self.voice = Voice {
            curve,
            gain: amount,
            norm,
            ja: JaParams::default(),
        };
        self.voice_key = key;
        self.voice
    }

    /// Process one stereo block in place. `viz`, when given, receives the
    /// block's meter levels.
    pub fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        s: &Settings,
        viz: Option<&ColorViz>,
    ) {
        let n = left.len().min(right.len());
        if self.configured.as_ref() != Some(s) {
            self.configure(s);
        }

        self.drive_s.set_target(s.drive);
        self.bias_s.set_target(s.bias);
        self.mix_s.set_target(s.mix.clamp(0.0, 1.0));
        self.output_s.set_target(s.output_db);
        self.auto_s.set_target(if s.auto_gain { 1.0 } else { 0.0 });
        self.response_s.set_target(s.response_db);
        self.tone_s.set_target(s.tone_db);
        // Filter coefficients are block-rate: land them on where the
        // smoothers will be at the end of this block.
        self.response_s.skip(n as u32);
        self.tone_s.skip(n as u32);
        self.apply_filters(s.stage_factor());

        let flutter_on = s.mode == Mode::Tape && s.flutter > 0.0;
        if s.flutter > 0.0 {
            self.flutter_held = s.flutter.min(1.0);
        }
        self.flutter_s.set_target(self.flutter_held);

        let mode = s.mode;
        let flags = self.flags;
        let mut in_peak = self.in_peak;
        let mut out_peak = self.out_peak;
        let decay = self.meter_decay.powi(n as i32);
        in_peak *= decay;
        out_peak *= decay;

        for i in 0..n {
            // A NaN or an infinity would lodge in the oversamplers'
            // allpass state and every biquad for good; read it as silence.
            let x = [sanitize(left[i]), sanitize(right[i])];
            let drive = self.drive_s.next();
            let bias = self.bias_s.next();
            let voice = self.voice_for(mode, flags.hq, drive, bias);

            let (d0, w0) = self.ch[0].tick(x[0], &voice, mode, &flags);
            let (d1, w1) = self.ch[1].tick(x[1], &voice, mode, &flags);
            let dry = [d0, d1];
            let wet = [w0, w1];

            let g_match = self.auto.gain(dry, wet);
            let blend = self.auto_s.next();
            let g = if blend == 0.0 {
                1.0
            } else if blend == 1.0 {
                g_match
            } else {
                1.0 + blend * (g_match - 1.0)
            };
            self.last_auto_gain = g;

            let mix = self.mix_s.next();
            let out_db = self.output_s.next();
            let out_gain = if out_db == 0.0 { 1.0 } else { db_to_linear(out_db) };

            // Flutter crossfade.
            if flutter_on {
                self.flutter_fade = (self.flutter_fade + self.flutter_step).min(1.0);
            } else {
                self.flutter_fade = (self.flutter_fade - self.flutter_step).max(0.0);
            }
            let fade = self.flutter_fade;
            let fl_amount = self.flutter_s.next();

            for c in 0..2 {
                let w = if g == 1.0 { wet[c] } else { wet[c] * g };
                let mut y = if mix == 0.0 {
                    dry[c]
                } else if mix == 1.0 {
                    w
                } else {
                    dry[c] * (1.0 - mix) + w * mix
                };
                if out_gain != 1.0 {
                    y *= out_gain;
                }
                let fl = &mut self.ch[c].flutter;
                if fade == 0.0 {
                    // Bypassed: returns `y` untouched but keeps recording,
                    // so switching on has history to read.
                    fl.set_amount(0.0);
                    y = fl.process(y);
                } else {
                    fl.set_amount(fl_amount.max(1.0e-6));
                    let f = fl.process(y);
                    y = if fade >= 1.0 { f } else { y + fade * (f - y) };
                }
                if c == 0 {
                    in_peak = in_peak.max(x[0].abs());
                    out_peak = out_peak.max(y.abs());
                    left[i] = y;
                } else {
                    in_peak = in_peak.max(x[1].abs());
                    out_peak = out_peak.max(y.abs());
                    right[i] = y;
                }
            }
        }

        self.in_peak = if in_peak.is_finite() { in_peak } else { 0.0 };
        self.out_peak = if out_peak.is_finite() { out_peak } else { 0.0 };
        if let Some(viz) = viz {
            viz.store(
                linear_to_db(self.in_peak),
                linear_to_db(self.out_peak),
                linear_to_db(self.last_auto_gain),
            );
        }
    }

    /// The auto-gain the last sample applied to the wet path, in dB.
    pub fn auto_gain_db(&self) -> f32 {
        linear_to_db(self.last_auto_gain)
    }
}
