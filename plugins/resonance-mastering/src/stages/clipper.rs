//! Peak clipper, between the imager and the limiter.
//!
//! Shaving the top 1–3 dB of transients before the limiter means the
//! limiter has less to do and pumps less (warmth-width-depth.md §3.1,
//! §3.4). The curve is `resonance_dsp`'s [`Curve::Clip`], a soft↔hard
//! clipper with a ±1 ceiling and unity gain below its knee, run through
//! first-order ADAA ([`Adaa1`]).
//!
//! # Drive and level
//!
//! `drive` pushes the signal into the curve and the output is divided by
//! it again, so material below the knee passes at unity and the ceiling
//! sits at `−drive` dBFS: a transient peaking at 0 dBFS comes out
//! `drive` dB lower, and everything quieter than the knee is untouched.
//! The stage never adds loudness by itself; the gain that uses the
//! headroom it makes belongs upstream (input trim, glue make-up), and the
//! limiter after it catches what is left.
//!
//! # Aliasing
//!
//! A hard clip has a slope corner, and its harmonics run far past
//! Nyquist. The curve therefore runs at 8×: the latency-free IIR
//! [`Oversampler`] cascaded, a 2× instance around a 4× one (the recipe
//! the `saturate` module documents). With ADAA that keeps the strongest
//! alias of a 5 kHz full-scale tone at +12 dB drive ≤ −90 dBc, where the
//! 4× alone reaches only about −73 dBc.
//!
//! # Delay
//!
//! The IIR half-bands have no fixed delay, so [`Clipper::latency`] is 0
//! and the chain's reported latency does not change. They do delay the
//! wet path by a frequency-dependent amount: about 6.9 samples at 48 kHz
//! from DC to a few kHz (4.0 from the outer 2× pair, 2.8 from the inner
//! 4× one at twice the rate), which the host's delay compensation does
//! not see. The enable crossfade (10 ms) mixes that wet path with the
//! raw input, so it combs while it runs (first notch ≈3.5 kHz at its
//! midpoint). The dry side is deliberately *not* run through a matching
//! pair: the fade would then start and end on a step between the
//! delayed and the raw signal, a click where the comb is only a brief
//! phase smear. See the chain's module docs.
//!
//! Off (after its fade-out) the stage is a wire, bit for bit.

use resonance_dsp::{db_to_linear, Adaa1, Curve, OversampleFactor, Oversampler};
use resonance_plugin::{Smoother, SmoothingStyle};

use super::retarget;

/// Ramp length for drive / shape and the enable crossfade, in ms.
const RAMP_MS: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipperConfig {
    pub enabled: bool,
    /// How far the signal is pushed into the ceiling, in dB. The ceiling
    /// then sits at `-drive_db` dBFS (see the module docs).
    pub drive_db: f32,
    /// 0 = hard clip, 1 = the softest knee.
    pub softness: f32,
}

impl Default for ClipperConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            drive_db: 2.0,
            softness: 0.0,
        }
    }
}

impl ClipperConfig {
    /// The `resonance_dsp` curve for a softness (0 hard … 1 soft).
    pub fn curve(softness: f32) -> Curve {
        Curve::Clip {
            shape: 1.0 - softness.clamp(0.0, 1.0),
        }
    }
}

/// One channel of the 8× clip: outer 2× around inner 4×, ADAA inside.
#[derive(Clone, Copy)]
struct ClipChannel {
    outer: Oversampler,
    inner: Oversampler,
    adaa: Adaa1,
}

impl ClipChannel {
    fn new() -> Self {
        let mut outer = Oversampler::new();
        outer.set_factor(OversampleFactor::X2);
        let mut inner = Oversampler::new();
        inner.set_factor(OversampleFactor::X4);
        Self {
            outer,
            inner,
            adaa: Adaa1::new(),
        }
    }

    fn reset(&mut self) {
        self.outer.reset();
        self.inner.reset();
        self.adaa.reset();
    }

    /// Clip the driven sample `u` at 8× and return it at the base rate.
    #[inline]
    fn process(&mut self, u: f32, curve: &Curve) -> f32 {
        let mut mid = self.outer.upsample(u);
        for m in &mut mid[..2] {
            let mut hi = self.inner.upsample(*m);
            for v in &mut hi {
                *v = self.adaa.process(curve, *v);
            }
            *m = self.inner.downsample(&hi);
        }
        self.outer.downsample(&mid)
    }
}

pub struct Clipper {
    left: ClipChannel,
    right: ClipChannel,
    drive_sm: Smoother,
    drive_tgt: f32,
    softness_sm: Smoother,
    softness_tgt: f32,
    enable_sm: Smoother,
    enable_tgt: f32,
    /// Linear drive for `cached_drive_db` (recomputed only while the
    /// drive ramps).
    cached_drive_db: f32,
    drive_lin: f32,
    inv_drive: f32,
    primed: bool,
    was_enabled: bool,
}

impl Clipper {
    pub fn new(sample_rate: f32) -> Self {
        let sm = || {
            let mut s = Smoother::new(SmoothingStyle::Linear(RAMP_MS));
            s.set_sample_rate(sample_rate);
            s
        };
        Self {
            left: ClipChannel::new(),
            right: ClipChannel::new(),
            drive_sm: sm(),
            drive_tgt: f32::NAN,
            softness_sm: sm(),
            softness_tgt: f32::NAN,
            enable_sm: sm(),
            enable_tgt: f32::NAN,
            cached_drive_db: f32::NAN,
            drive_lin: 1.0,
            inv_drive: 1.0,
            primed: false,
            was_enabled: false,
        }
    }

    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
        self.drive_sm.reset(0.0);
        self.drive_tgt = f32::NAN;
        self.softness_sm.reset(0.0);
        self.softness_tgt = f32::NAN;
        self.enable_sm.reset(0.0);
        self.enable_tgt = f32::NAN;
        self.cached_drive_db = f32::NAN;
        self.primed = false;
        self.was_enabled = false;
    }

    /// Zero: the IIR oversampling has no fixed delay. Its ~6.9-sample
    /// frequency-dependent group delay is not included (module docs).
    pub fn latency(&self) -> usize {
        0
    }

    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32], cfg: &ClipperConfig) {
        let drive_db = cfg.drive_db.max(0.0);
        let softness = cfg.softness.clamp(0.0, 1.0);
        if cfg.enabled && !self.was_enabled {
            // (Re)engage from a clean state; the enable crossfade covers
            // the oversamplers settling.
            self.left.reset();
            self.right.reset();
            self.drive_sm.reset(drive_db);
            self.drive_tgt = drive_db;
            self.softness_sm.reset(softness);
            self.softness_tgt = softness;
            if !self.primed {
                self.enable_sm.reset(1.0);
                self.enable_tgt = 1.0;
            }
        } else {
            retarget(&mut self.drive_sm, &mut self.drive_tgt, drive_db);
            retarget(&mut self.softness_sm, &mut self.softness_tgt, softness);
        }
        let enable_target = if cfg.enabled { 1.0 } else { 0.0 };
        retarget(&mut self.enable_sm, &mut self.enable_tgt, enable_target);
        self.was_enabled = cfg.enabled;
        self.primed = true;

        if !cfg.enabled && self.enable_sm.current() == 0.0 {
            return;
        }

        let frames = left.len().min(right.len());
        for i in 0..frames {
            let d = self.drive_sm.next();
            if d != self.cached_drive_db {
                self.cached_drive_db = d;
                self.drive_lin = db_to_linear(d);
                self.inv_drive = 1.0 / self.drive_lin;
            }
            let curve = ClipperConfig::curve(self.softness_sm.next());
            let e = self.enable_sm.next();
            let (dl, dr) = (left[i], right[i]);
            let wl = self.left.process(dl * self.drive_lin, &curve) * self.inv_drive;
            let wr = self.right.process(dr * self.drive_lin, &curve) * self.inv_drive;
            left[i] = dl + (wl - dl) * e;
            right[i] = dr + (wr - dr) * e;
        }
    }
}
