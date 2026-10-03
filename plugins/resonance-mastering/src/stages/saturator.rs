//! Tape / tube saturator.
//!
//! Intended for mastering-grade harmonic coloration. `sat_mode` picks
//! the voicing: the default, [`SatMode::Blend`], is the original
//! Tube↔Tape blend described below, and is unchanged by the other modes
//! existing — a project saved before them renders bit-identically. The
//! other modes (Tube, Tape, Transformer, Console, Warm, Inflator) are in
//! [`super::sat_modes`].
//!
//! # The Blend mode
//!
//! Two shaper modes:
//! `Smooth` runs `tanh` for clean even/odd harmonics; `Gritty` runs a
//! cubic soft-clipper with a sharper knee and richer odd-harmonic
//! content for a more obviously analog sound.
//!
//! Both shapers run through first-order antiderivative antialiasing
//! (ADAA). A memoryless nonlinearity generates harmonics with no upper
//! bound — the cubic hard-clamps at its knee, a C¹ corner whose
//! harmonic series extends far past Nyquist on full-band material, and
//! `tanh` at +18 dB drive is nearly as bright — and every partial born
//! above Nyquist folds back into the passband as *inharmonic* grit.
//! Instead of oversampling, each output sample is the exact average of
//! the shaper over the segment the driven input traversed since the
//! previous sample, `(F(u[n]) − F(u[n−1])) / (u[n] − u[n−1])` with `F`
//! the shaper's closed-form antiderivative; that continuous-time
//! averaging acts as an extra first-order lowpass on the distortion
//! products and knocks the folded partials down steeply with
//! frequency. ADAA runs in the *driven*-input domain `u = drive · x`
//! (the nonlinearity is static in `u`, so the antiderivatives stay
//! valid even while the drive smoother ramps per-sample). It adds no
//! latency to the chain's latency model — the nonlinear path acquires
//! only a ~half-sample *effective* delay, which the internal dry/wet
//! mix tolerates (its worst case is a gentle ~3 dB shade at Nyquist at
//! mix = 0.5, and the wet path is already phase-shifted by the two
//! shelves anyway).
//!
//! Chain per sample:
//!
//!   dry → HF shelf cut (tape loss) → waveshaper(drive) → DC blocker
//!   → LF shelf boost (head bump) → peak-normalize → mix(dry, wet)
//!
//! Normalization divides by the shaper's value at full drive, not by
//! drive itself: that keeps full-scale peaks pinned near unity at any
//! drive setting while quiet content receives an automatic makeup
//! boost, so pushing the drive knob audibly *adds* saturation instead
//! of just attenuating peaks.
//!
//! The waveshaper crossfades a symmetric variant (odd harmonics only)
//! against an asymmetric one (DC-offset before the shaper, then the
//! offset's own shaped value subtracted to pass through the origin),
//! producing 2nd-harmonic content as the character knob moves toward
//! tape. The asymmetric branch leaves the output with a nonzero mean,
//! so a DC blocker runs right after the shaper — always, not just at
//! character > 0, so the wet path stays continuous as the knob sweeps —
//! before the low shelf can amplify the offset.

use resonance_dsp::{db_to_linear, Biquad, Curve, DcBlocker};
use resonance_plugin::{Smoother, SmoothingStyle};

use super::retarget;
pub use super::sat_modes::SatMode;
use super::sat_modes::{peak_gain, ModeChannel};

/// Ramp length for drive/character/mix and the enable crossfade, in
/// milliseconds. Long enough to spread a full-scale parameter step
/// over ~480 samples at 48 kHz (no click), short enough to still feel
/// instant under the knob.
const RAMP_MS: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shaper {
    /// `tanh`-based soft clipper. Clean, mostly low-order harmonics.
    Smooth,
    /// Cubic soft clipper. Sharper knee, richer odd-harmonic content.
    Gritty,
}

impl Shaper {
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Shaper::Gritty,
            _ => Shaper::Smooth,
        }
    }
    pub fn to_index(self) -> i32 {
        match self {
            Shaper::Smooth => 0,
            Shaper::Gritty => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SaturatorConfig {
    pub enabled: bool,
    /// Input drive in dB. 0..18 is a reasonable range.
    pub drive_db: f32,
    /// 0.0 = fully symmetric (odd harmonics), 1.0 = fully asymmetric (adds 2nd harmonic).
    pub character: f32,
    /// Dry/wet mix.
    pub mix: f32,
    /// Which waveshaper to run (Blend mode).
    pub shaper: Shaper,
    /// Which voicing to run; [`SatMode::Blend`] is the original stage.
    pub mode: SatMode,
    /// The Inflator's Curve control, −0.5..0.5 (the JSFX's ±50 %).
    pub curve: f32,
}

impl Default for SaturatorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            drive_db: 3.0,
            character: 0.3,
            mix: 1.0,
            shaper: Shaper::Smooth,
            mode: SatMode::Blend,
            curve: 0.0,
        }
    }
}

pub struct Saturator {
    sample_rate: f32,
    // Per-channel biquads so the L/R states stay independent.
    hf_shelf_l: Biquad,
    hf_shelf_r: Biquad,
    lf_shelf_l: Biquad,
    lf_shelf_r: Biquad,
    dc_l: DcBlocker,
    dc_r: DcBlocker,
    /// Per-sample smoothers so parameter steps and the enable toggle
    /// ramp instead of clicking. `*_tgt` mirrors the last requested
    /// target (see [`retarget`]).
    drive_sm: Smoother,
    drive_tgt: f32,
    character_sm: Smoother,
    character_tgt: f32,
    mix_sm: Smoother,
    mix_tgt: f32,
    /// Enable crossfade: multiplies the wet mix, ramping 0 ↔ 1 on
    /// toggles so switching the stage mid-signal fades rather than
    /// steps.
    enable_sm: Smoother,
    enable_tgt: f32,
    /// Drive-derived values, recomputed only when the smoothed drive
    /// or the shaper actually changes (NaN forces the first compute).
    cached_drive_db: f32,
    cached_shaper: Shaper,
    drive_lin: f32,
    inv_drive: f32,
    /// ADAA memory: the previous *driven* shaper input `u[n−1] =
    /// drive_lin·x[n−1]`, one per channel so L/R stay independent.
    /// Held in f64 because the ADAA quotient subtracts two nearby
    /// antiderivative values; f32 cancellation there would put noise
    /// on the master bus.
    adaa_x1_l: f64,
    adaa_x1_r: f64,
    /// Has any audio streamed through since construction/reset? An
    /// enable on the very first block engages instantly (there is no
    /// audio history to click against); a later enable crossfades in.
    primed: bool,
    was_enabled: bool,

    /// The non-Blend modes' per-channel paths (see `sat_modes`).
    mode_l: ModeChannel,
    mode_r: ModeChannel,
    /// The Inflator Curve control, smoothed like the other params.
    curve_sm: Smoother,
    curve_tgt: f32,
    /// The mode the stage state belongs to; a change restarts it.
    active_mode: SatMode,
    /// Mode-switch fade (DSP2-11): multiplies the wet share like the
    /// enable crossfade. A `sat_mode` change ramps it to 0 on the old
    /// mode, swaps modes (restarting their state) once it is there, and
    /// ramps back to 1 on the new one — instead of jumping from one
    /// voicing's output to a freshly restarted other one in one sample.
    switch_sm: Smoother,
    switch_tgt: f32,
    /// Drive (dB) and curve the cached mode values were computed for
    /// (NaN forces the first compute).
    mode_key: (f32, f32),
    mode_drive_lin: f32,
    mode_gain: f32,
    mode_curve: Curve,
}

impl Saturator {
    pub fn new(sample_rate: f32) -> Self {
        let mut s = Self {
            sample_rate,
            hf_shelf_l: Biquad::identity(),
            hf_shelf_r: Biquad::identity(),
            lf_shelf_l: Biquad::identity(),
            lf_shelf_r: Biquad::identity(),
            dc_l: DcBlocker::default(),
            dc_r: DcBlocker::default(),
            drive_sm: Smoother::new(SmoothingStyle::Linear(RAMP_MS)),
            drive_tgt: f32::NAN,
            character_sm: Smoother::new(SmoothingStyle::Linear(RAMP_MS)),
            character_tgt: f32::NAN,
            mix_sm: Smoother::new(SmoothingStyle::Linear(RAMP_MS)),
            mix_tgt: f32::NAN,
            enable_sm: Smoother::new(SmoothingStyle::Linear(RAMP_MS)),
            enable_tgt: f32::NAN,
            cached_drive_db: f32::NAN,
            cached_shaper: Shaper::Smooth,
            drive_lin: 1.0,
            inv_drive: 1.0,
            adaa_x1_l: 0.0,
            adaa_x1_r: 0.0,
            primed: false,
            was_enabled: false,
            mode_l: ModeChannel::new(sample_rate),
            mode_r: ModeChannel::new(sample_rate),
            curve_sm: Smoother::new(SmoothingStyle::Linear(RAMP_MS)),
            curve_tgt: f32::NAN,
            active_mode: SatMode::Blend,
            switch_sm: Smoother::new(SmoothingStyle::Linear(RAMP_MS)),
            switch_tgt: 1.0,
            mode_key: (f32::NAN, f32::NAN),
            mode_drive_lin: 1.0,
            mode_gain: 1.0,
            mode_curve: Curve::Tanh,
        };
        s.switch_sm.reset(1.0);
        s.set_sample_rate(sample_rate);
        s
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        // Tape-style HF loss: -3 dB shelf starting at 14 kHz.
        self.hf_shelf_l
            .set_high_shelf(sample_rate, 14_000.0, 0.707, -3.0);
        self.hf_shelf_r
            .set_high_shelf(sample_rate, 14_000.0, 0.707, -3.0);
        // Tape head bump: +2 dB low shelf around 100 Hz.
        self.lf_shelf_l
            .set_low_shelf(sample_rate, 100.0, 0.707, 2.0);
        self.lf_shelf_r
            .set_low_shelf(sample_rate, 100.0, 0.707, 2.0);
        // The wet path is the whole master at mix 1.0: keep the DC
        // corner at 5 Hz in Hz, not in samples (DSP-04).
        self.dc_l
            .set_cutoff(DcBlocker::DEFAULT_CUTOFF_HZ, sample_rate);
        self.dc_r
            .set_cutoff(DcBlocker::DEFAULT_CUTOFF_HZ, sample_rate);
        self.drive_sm.set_sample_rate(sample_rate);
        self.character_sm.set_sample_rate(sample_rate);
        self.mix_sm.set_sample_rate(sample_rate);
        self.enable_sm.set_sample_rate(sample_rate);
        self.curve_sm.set_sample_rate(sample_rate);
        self.switch_sm.set_sample_rate(sample_rate);
        self.mode_l = ModeChannel::new(sample_rate);
        self.mode_r = ModeChannel::new(sample_rate);
    }

    pub fn reset(&mut self) {
        self.hf_shelf_l.reset();
        self.hf_shelf_r.reset();
        self.lf_shelf_l.reset();
        self.lf_shelf_r.reset();
        self.dc_l.reset();
        self.dc_r.reset();
        self.drive_sm.reset(0.0);
        self.drive_tgt = f32::NAN;
        self.character_sm.reset(0.0);
        self.character_tgt = f32::NAN;
        self.mix_sm.reset(0.0);
        self.mix_tgt = f32::NAN;
        self.enable_sm.reset(0.0);
        self.enable_tgt = f32::NAN;
        self.cached_drive_db = f32::NAN;
        self.adaa_x1_l = 0.0;
        self.adaa_x1_r = 0.0;
        self.primed = false;
        self.was_enabled = false;
        self.mode_l.reset();
        self.mode_r.reset();
        self.curve_sm.reset(0.0);
        self.curve_tgt = f32::NAN;
        self.mode_key = (f32::NAN, f32::NAN);
        self.switch_sm.reset(1.0);
        self.switch_tgt = 1.0;
    }

    /// Restart the filter and ADAA state of both paths: on (re)engage,
    /// and when the mode changes (the state belongs to the old mode).
    fn restart_state(&mut self) {
        self.hf_shelf_l.reset();
        self.hf_shelf_r.reset();
        self.lf_shelf_l.reset();
        self.lf_shelf_r.reset();
        self.dc_l.reset();
        self.dc_r.reset();
        self.adaa_x1_l = 0.0;
        self.adaa_x1_r = 0.0;
        self.mode_l.reset();
        self.mode_r.reset();
        self.mode_key = (f32::NAN, f32::NAN);
    }

    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32], cfg: &SaturatorConfig) {
        let character = cfg.character.clamp(0.0, 1.0);
        let mix = cfg.mix.clamp(0.0, 1.0);
        let curve = cfg.curve.clamp(-0.5, 0.5);

        // Re-enabled while the fade-out still runs: the wet path is live,
        // so the fade just turns around. Restarting its state under a
        // half-open fade would step the output.
        let fading_out = self.enable_sm.current() > 0.0;
        if cfg.enabled && !self.was_enabled && !fading_out {
            // (Re)engage. The parameter smoothers snap to the current
            // values — ramping in from whatever they held when the
            // stage was last audible would be meaningless — and the
            // filters restart clean, their state from that era being
            // equally stale; the enable crossfade covers the settling.
            self.hf_shelf_l.reset();
            self.hf_shelf_r.reset();
            self.lf_shelf_l.reset();
            self.lf_shelf_r.reset();
            self.dc_l.reset();
            self.dc_r.reset();
            // The ADAA memory is state from the same stale era as the
            // filters, so it resets with them: once the stage has been
            // fully faded out its `u[n−1]` stops tracking the input
            // (the early return below skips processing entirely), and
            // an ADAA step from a months-old sample would smear the
            // first wet sample arbitrarily. The reset's own first-step
            // error is bounded — the ADAA quotient is a mean of f, so
            // |output| ≤ sup|f| always — and it lands together with
            // the filter resets under the same enable crossfade, so it
            // is no more audible than they are.
            self.adaa_x1_l = 0.0;
            self.adaa_x1_r = 0.0;
            self.drive_sm.reset(cfg.drive_db);
            self.drive_tgt = cfg.drive_db;
            self.character_sm.reset(character);
            self.character_tgt = character;
            self.mix_sm.reset(mix);
            self.mix_tgt = mix;
            self.mode_l.reset();
            self.mode_r.reset();
            self.curve_sm.reset(curve);
            self.curve_tgt = curve;
            if !self.primed {
                // Very first block: engage instantly, there is no
                // running audio to click against.
                self.enable_sm.reset(1.0);
                self.enable_tgt = 1.0;
            }
        } else {
            retarget(&mut self.drive_sm, &mut self.drive_tgt, cfg.drive_db);
            retarget(&mut self.character_sm, &mut self.character_tgt, character);
            retarget(&mut self.mix_sm, &mut self.mix_tgt, mix);
            retarget(&mut self.curve_sm, &mut self.curve_tgt, curve);
        }
        if cfg.mode != self.active_mode {
            // Silent (never ran, or fully faded out): swap at once.
            // Otherwise fade the old mode out first; the swap happens on
            // the block where that fade has landed.
            // `fading_out`: the wet path was audible in the previous block.
            let audible = fading_out;
            if !audible || self.switch_sm.current() == 0.0 {
                self.restart_state();
                self.active_mode = cfg.mode;
                if audible {
                    retarget(&mut self.switch_sm, &mut self.switch_tgt, 1.0);
                } else {
                    self.switch_sm.reset(1.0);
                    self.switch_tgt = 1.0;
                }
            } else {
                retarget(&mut self.switch_sm, &mut self.switch_tgt, 0.0);
            }
        } else {
            retarget(&mut self.switch_sm, &mut self.switch_tgt, 1.0);
        }
        let enable_target = if cfg.enabled { 1.0 } else { 0.0 };
        retarget(&mut self.enable_sm, &mut self.enable_tgt, enable_target);
        self.was_enabled = cfg.enabled;
        self.primed = true;

        // Fully faded out: the stage is a wire, bit-identical to a
        // hard bypass once the disable crossfade has finished.
        if !cfg.enabled && self.enable_sm.current() == 0.0 {
            return;
        }

        if self.active_mode != SatMode::Blend {
            self.process_mode(left, right, self.active_mode);
            return;
        }

        let shaper = cfg.shaper;
        let frames = left.len().min(right.len());
        for i in 0..frames {
            let dry_l = left[i];
            let dry_r = right[i];

            // Peak-normalize: divide by the shaper's value at full
            // drive so a 1.0-amplitude input pins to ~1.0 regardless
            // of drive. Recomputed only while the drive ramp is live
            // (or the shaper switched); converged blocks reuse the
            // cache.
            let drive_db = self.drive_sm.next();
            if drive_db != self.cached_drive_db || shaper != self.cached_shaper {
                self.cached_drive_db = drive_db;
                self.cached_shaper = shaper;
                self.drive_lin = db_to_linear(drive_db);
                self.inv_drive =
                    (1.0 / base_shape(self.drive_lin as f64, shaper).max(1e-6)) as f32;
            }
            let drive = self.drive_lin;
            let inv_drive = self.inv_drive;
            let character = self.character_sm.next() as f64;
            let mix = self.mix_sm.next() * self.enable_sm.next() * self.switch_sm.next();

            let l1 = self.hf_shelf_l.process(dry_l);
            let r1 = self.hf_shelf_r.process(dry_r);

            // ADAA in the driven domain: the smoothed drive is folded
            // into `u` before the shaper, so per-sample drive ramps
            // just move this sample's segment endpoint — the
            // antiderivatives themselves never depend on drive.
            let ul = l1 as f64 * drive as f64;
            let ur = r1 as f64 * drive as f64;
            let wet_l = (waveshape_adaa(ul, self.adaa_x1_l, character, shaper) as f32) * inv_drive;
            let wet_r = (waveshape_adaa(ur, self.adaa_x1_r, character, shaper) as f32) * inv_drive;
            self.adaa_x1_l = ul;
            self.adaa_x1_r = ur;

            let l2 = self.dc_l.process(wet_l);
            let r2 = self.dc_r.process(wet_r);

            let l3 = self.lf_shelf_l.process(l2);
            let r3 = self.lf_shelf_r.process(r2);

            left[i] = dry_l + (l3 - dry_l) * mix;
            right[i] = dry_r + (r3 - dry_r) * mix;
        }
    }
}

impl Saturator {
    /// The non-Blend modes (see `sat_modes`): each channel's dry/wet
    /// blend at 4×, then the enable crossfade against the raw input.
    fn process_mode(&mut self, left: &mut [f32], right: &mut [f32], mode: SatMode) {
        let frames = left.len().min(right.len());
        for i in 0..frames {
            let drive_db = self.drive_sm.next();
            let curve = self.curve_sm.next();
            if (drive_db, curve) != self.mode_key || self.mode_key.0.is_nan() {
                self.mode_key = (drive_db, curve);
                self.mode_drive_lin = db_to_linear(drive_db);
                self.mode_gain = peak_gain(mode, self.mode_drive_lin, curve);
                self.mode_curve = mode.curve(self.mode_drive_lin, curve).unwrap_or(Curve::Tanh);
            }
            // Kept moving so it has converged if the mode goes back to
            // Blend.
            let _ = self.character_sm.next();
            let mix = self.mix_sm.next();
            let e = self.enable_sm.next() * self.switch_sm.next();
            let (dl, dr) = (left[i], right[i]);
            let (drive, gain, c) = (self.mode_drive_lin, self.mode_gain, self.mode_curve);
            let wl = self.mode_l.process(mode, &c, dl, drive, gain, mix);
            let wr = self.mode_r.process(mode, &c, dr, drive, gain, mix);
            left[i] = dl + (wl - dl) * e;
            right[i] = dr + (wr - dr) * e;
        }
    }
}

/// Underlying soft-clip curve. `Smooth` is `tanh`; `Gritty` is a
/// scaled cubic clipper (`x - x³/3` past a threshold, hard-clipped at
/// ±1) which transitions from linear to clipped much faster than
/// `tanh` and produces noticeably more harmonic content at the same
/// input level.
#[inline]
fn base_shape(x: f64, shaper: Shaper) -> f64 {
    match shaper {
        Shaper::Smooth => x.tanh(),
        Shaper::Gritty => {
            // Scale so the linear region has unit slope at x=0 and the
            // curve saturates near ±1. The cubic 1.5·(u - u³/3) at
            // u = x/1.5 has slope 1 at zero and reaches 1.0 at u = 1.
            let u = (x / 1.5).clamp(-1.0, 1.0);
            1.5 * (u - (u * u * u) / 3.0)
        }
    }
}

/// Closed-form antiderivative `F` of [`base_shape`], `F′ = f`. The
/// integration constant is irrelevant (ADAA only ever takes
/// differences of `F`) but `F` must be *continuous*, including across
/// the cubic's clamp points — a jump there would put a spike in every
/// output sample whose input segment crosses the knee.
#[inline]
fn shape_antiderivative(x: f64, shaper: Shaper) -> f64 {
    match shaper {
        Shaper::Smooth => {
            // ∫ tanh(x) dx = ln cosh(x). Evaluated as
            //   ln cosh(x) = |x| + ln(1 + e^{−2|x|}) − ln 2
            // which never overflows (cosh itself blows up past x ≈ 700
            // and drive alone reaches ~8 here, so hot inter-sample
            // segments would be at risk in the naive form).
            let ax = x.abs();
            ax + (-2.0 * ax).exp().ln_1p() - std::f64::consts::LN_2
        }
        Shaper::Gritty => {
            // Piecewise, matching the clamp in `base_shape`:
            //   |x| ≤ 1.5:  f(x) = x − (4/27)x³   (the cubic in x-units:
            //               1.5·(u − u³/3) with u = x/1.5)
            //               F(x) = x²/2 − x⁴/27
            //   |x| > 1.5:  f(x) = sign(x)·1
            //               F(x) = |x| − 9/16
            // Continuity at the knee: F(±1.5) = 1.125 − 0.1875 = 0.9375
            // from the cubic branch and 1.5 − 0.5625 = 0.9375 from the
            // clamped branch. (F is even because f is odd.)
            let ax = x.abs();
            if ax <= 1.5 {
                let x2 = x * x;
                x2 / 2.0 - x2 * x2 / 27.0
            } else {
                ax - 0.5625
            }
        }
    }
}

/// Below this input step the ADAA quotient `(F(u0) − F(u1)) / (u0 − u1)`
/// is a 0/0 and the code falls back to the midpoint rule
/// `f((u0 + u1)/2)`. The two forms agree to O(Δu²), so the switch is
/// seamless; in f64 the quotient itself is still accurate to ~1e-10 at
/// this threshold, so the exact value is uncritical.
const ADAA_EPS: f64 = 1.0e-5;

/// First-order ADAA evaluation of [`base_shape`] over the segment
/// `[u1, u0]`: the exact mean of `f` across the interval the driven
/// input traversed, which is what suppresses the fold-back of
/// harmonics born above Nyquist. `du = u0 − u1` is passed in so the
/// symmetric and offset (asymmetric) branches share one denominator
/// and one fallback decision.
#[inline]
fn adaa1(u0: f64, u1: f64, du: f64, shaper: Shaper) -> f64 {
    if du.abs() < ADAA_EPS {
        base_shape(0.5 * (u0 + u1), shaper)
    } else {
        (shape_antiderivative(u0, shaper) - shape_antiderivative(u1, shaper)) / du
    }
}

/// ADAA counterpart of the memoryless waveshaper: `u0` is the current
/// driven input, `u1` the previous one (per channel).
#[inline]
fn waveshape_adaa(u0: f64, u1: f64, character: f64, shaper: Shaper) -> f64 {
    let du = u0 - u1;
    // Symmetric branch: pure odd harmonics.
    let symmetric = adaa1(u0, u1, du, shaper);
    // Asymmetric branch: DC-offset before the shaper, then subtract
    // the offset's own shaped value so the curve still passes through
    // the origin. The tilted transfer function generates 2nd-harmonic
    // content. Larger offset → more obvious tube/tape character.
    // Offsetting both endpoints leaves du unchanged, so the branch
    // shares the symmetric branch's denominator; the subtracted
    // `f(offset)` is a constant and needs no antialiasing.
    let offset = 0.35_f64;
    let asymmetric = adaa1(u0 + offset, u1 + offset, du, shaper) - base_shape(offset, shaper);
    symmetric * (1.0 - character) + asymmetric * character
}

