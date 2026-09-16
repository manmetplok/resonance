//! Tape / tube saturator.
//!
//! Intended for mastering-grade harmonic coloration. Two shaper modes:
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
//! the shaper over the segment the input traversed since the previous
//! sample, `(F(x[n]) − F(x[n−1])) / (x[n] − x[n−1])` with `F` the
//! shaper's closed-form antiderivative; that continuous-time averaging
//! acts as an extra first-order lowpass on the distortion products and
//! knocks the folded partials down steeply with frequency. It adds no
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

use resonance_dsp::{db_to_linear, Biquad, DcBlocker};

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
    /// Which waveshaper to run.
    pub shaper: Shaper,
}

impl Default for SaturatorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            drive_db: 3.0,
            character: 0.3,
            mix: 1.0,
            shaper: Shaper::Smooth,
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
    // ADAA memory: the previous *driven* shaper input (post drive
    // gain), one per channel so L/R stay independent. Held in f64
    // because the ADAA quotient subtracts two nearby antiderivative
    // values; f32 cancellation there would put noise on the master bus.
    adaa_x1_l: f64,
    adaa_x1_r: f64,
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
            adaa_x1_l: 0.0,
            adaa_x1_r: 0.0,
        };
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
    }

    pub fn reset(&mut self) {
        self.hf_shelf_l.reset();
        self.hf_shelf_r.reset();
        self.lf_shelf_l.reset();
        self.lf_shelf_r.reset();
        self.dc_l.reset();
        self.dc_r.reset();
        self.adaa_x1_l = 0.0;
        self.adaa_x1_r = 0.0;
    }

    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32], cfg: &SaturatorConfig) {
        if !cfg.enabled {
            return;
        }

        let drive = db_to_linear(cfg.drive_db) as f64;
        let shaper = cfg.shaper;
        // Peak-normalize: divide by the shaper's value at full drive
        // so a 1.0-amplitude input pins to ~1.0 regardless of drive.
        let inv_drive = 1.0 / base_shape(drive, shaper).max(1e-6);
        let character = cfg.character.clamp(0.0, 1.0) as f64;
        let mix = cfg.mix.clamp(0.0, 1.0);

        let frames = left.len().min(right.len());
        for i in 0..frames {
            let dry_l = left[i];
            let dry_r = right[i];

            let l1 = self.hf_shelf_l.process(dry_l);
            let r1 = self.hf_shelf_r.process(dry_r);

            let xl = l1 as f64 * drive;
            let xr = r1 as f64 * drive;
            let wet_l = waveshape_adaa(xl, self.adaa_x1_l, character, shaper) * inv_drive;
            let wet_r = waveshape_adaa(xr, self.adaa_x1_r, character, shaper) * inv_drive;
            self.adaa_x1_l = xl;
            self.adaa_x1_r = xr;

            let l2 = self.dc_l.process(wet_l as f32);
            let r2 = self.dc_r.process(wet_r as f32);

            let l3 = self.lf_shelf_l.process(l2);
            let r3 = self.lf_shelf_r.process(r2);

            left[i] = dry_l + (l3 - dry_l) * mix;
            right[i] = dry_r + (r3 - dry_r) * mix;
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

/// Below this input step the ADAA quotient `(F(x0) − F(x1)) / (x0 − x1)`
/// is a 0/0 and the code falls back to the midpoint rule
/// `f((x0 + x1)/2)`. The two forms agree to O(Δx²), so the switch is
/// seamless; in f64 the quotient itself is still accurate to ~1e-10 at
/// this threshold, so the exact value is uncritical.
const ADAA_EPS: f64 = 1.0e-5;

/// First-order ADAA evaluation of [`base_shape`] over the segment
/// `[x1, x0]`: the exact mean of `f` across the interval the input
/// traversed, which is what suppresses the fold-back of harmonics born
/// above Nyquist. `dx = x0 − x1` is passed in so the symmetric and
/// offset (asymmetric) branches share one denominator and one fallback
/// decision.
#[inline]
fn adaa1(x0: f64, x1: f64, dx: f64, shaper: Shaper) -> f64 {
    if dx.abs() < ADAA_EPS {
        base_shape(0.5 * (x0 + x1), shaper)
    } else {
        (shape_antiderivative(x0, shaper) - shape_antiderivative(x1, shaper)) / dx
    }
}

/// ADAA counterpart of the original memoryless waveshaper: `x0` is the
/// current driven input, `x1` the previous one (per channel).
#[inline]
fn waveshape_adaa(x0: f64, x1: f64, character: f64, shaper: Shaper) -> f64 {
    let dx = x0 - x1;
    // Symmetric branch: pure odd harmonics.
    let symmetric = adaa1(x0, x1, dx, shaper);
    // Asymmetric branch: DC-offset before the shaper, then subtract
    // the offset's own shaped value so the curve still passes through
    // the origin. The tilted transfer function generates 2nd-harmonic
    // content. Larger offset → more obvious tube/tape character.
    // Offsetting both endpoints leaves dx unchanged, so the branch
    // shares the symmetric branch's denominator; the subtracted
    // `f(offset)` is a constant and needs no antialiasing.
    let offset = 0.35_f64;
    let asymmetric = adaa1(x0 + offset, x1 + offset, dx, shaper) - base_shape(offset, shaper);
    symmetric * (1.0 - character) + asymmetric * character
}

