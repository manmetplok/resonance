//! Polyphase IIR half-band filters and a 1×/2×/4× oversampler built on
//! them, for running a nonlinearity (a waveshaper, a clipper) at a higher
//! rate so the harmonics it generates above Nyquist are filtered out
//! instead of folding back down as inharmonic aliases.
//!
//! # Why IIR allpass half-bands
//!
//! A half-band low-pass split into two polyphase branches, each a chain of
//! first-order allpass sections (Valenzuela & Constantinides; the structure
//! Laurent de Soras' HIIR popularised):
//!
//! ```text
//! H(z) = ½ · ( A₀(z²) + z⁻¹ · A₁(z²) )
//! ```
//!
//! Each branch runs at the *low* rate, so a 2× up- or down-sample costs one
//! multiply per coefficient per low-rate sample. Compared with a linear-
//! phase FIR of the same stopband it is several times cheaper and, more to
//! the point for a realtime stage, has **no fixed latency**: the response is
//! minimum-phase-like, with a small frequency-dependent group delay (about
//! four base-rate samples at low frequencies for the 2× round trip, five
//! and a half for 4×, measured in `tests/oversample.rs`) instead of the half-filter-length delay a FIR
//! imposes and would have to report to the host.
//!
//! The price is phase non-linearity near the band edge. That is inaudible
//! on a distortion stage, but it means the dry signal must be run through
//! the *same* up/down pair before it is mixed with the wet one (see
//! [`Oversampler`]'s docs), or a dry/wet blend comb-filters.
//!
//! Everything here is fixed-size: no allocation, no locks, and the filter
//! design (a few `sin`/`cos` in `f64`) runs in the constructors, which an
//! owner calls from `initialize`, never per sample.

use std::f64::consts::PI;

/// Allpass coefficients for a polyphase half-band of `coefs.len()` sections
/// with the given transition bandwidth.
///
/// `transition` is normalised to the **high** sample rate, in `(0, 0.25)`:
/// the passband ends at `(0.25 - transition)·fs` and the stopband starts at
/// `(0.25 + transition)·fs`. More coefficients or a wider transition give a
/// deeper stopband. Even-indexed coefficients belong to branch `A₀`, odd ones
/// to `A₁`.
///
/// This is the elliptic-filter design of Valenzuela & Constantinides
/// (1983), in the form de Soras' HIIR computes it.
pub fn halfband_coefs(coefs: &mut [f32], transition: f64) {
    let order = coefs.len() * 2 + 1;
    let (k, q) = transition_params(transition.clamp(1e-4, 0.2499));
    for (i, c) in coefs.iter_mut().enumerate() {
        *c = coef(i + 1, k, q, order) as f32;
    }
}

fn transition_params(transition: f64) -> (f64, f64) {
    let k = ((1.0 - transition * 2.0) * PI / 4.0).tan();
    let k = k * k;
    let kksqrt = (1.0 - k * k).powf(0.25);
    let e = 0.5 * (1.0 - kksqrt) / (1.0 + kksqrt);
    let e2 = e * e;
    let e4 = e2 * e2;
    let q = e * (1.0 + e4 * (2.0 + e4 * (15.0 + 150.0 * e4)));
    (k, q)
}

fn coef(c: usize, k: f64, q: f64, order: usize) -> f64 {
    let c = c as f64;
    let order = order as f64;

    // Numerator: Σ (-1)^i q^(i(i+1)) sin((2i+1)·c·π/order)
    let mut num = 0.0;
    let mut sign = 1.0;
    let mut i = 0i32;
    loop {
        let term = q.powi(i * (i + 1)) * ((i * 2 + 1) as f64 * c * PI / order).sin() * sign;
        num += term;
        sign = -sign;
        i += 1;
        if term.abs() <= 1e-100 || i > 64 {
            break;
        }
    }
    num *= q.powf(0.25);

    // Denominator: ½ + Σ (-1)^i q^(i²) cos(2i·c·π/order), i ≥ 1
    let mut den = 0.0;
    let mut sign = -1.0;
    let mut i = 1i32;
    loop {
        let term = q.powi(i * i) * ((i * 2) as f64 * c * PI / order).cos() * sign;
        den += term;
        sign = -sign;
        i += 1;
        if term.abs() <= 1e-100 || i > 64 {
            break;
        }
    }
    den += 0.5;

    let ww = num / den;
    let wwsq = ww * ww;
    let x = ((1.0 - wwsq * k) * (1.0 - wwsq / k)).sqrt() / (1.0 + wwsq);
    (1.0 - x) / (1.0 + x)
}

/// One polyphase half-band stage with `N` allpass sections, usable either
/// as a 2× upsampler or a 2× downsampler (use one instance per direction:
/// the state is per direction).
#[derive(Clone, Copy)]
pub struct Halfband<const N: usize> {
    coefs: [f32; N],
    x: [f32; N],
    y: [f32; N],
}

impl<const N: usize> Halfband<N> {
    /// A stage designed for `transition` (see [`halfband_coefs`]).
    pub fn new(transition: f64) -> Self {
        let mut coefs = [0.0; N];
        halfband_coefs(&mut coefs, transition);
        Self {
            coefs,
            x: [0.0; N],
            y: [0.0; N],
        }
    }

    /// The allpass coefficients in use (even index → branch 0).
    pub fn coefs(&self) -> &[f32; N] {
        &self.coefs
    }

    pub fn reset(&mut self) {
        self.x = [0.0; N];
        self.y = [0.0; N];
    }

    /// Run both branches one low-rate step: branch 0 on `a`, branch 1 on
    /// `b`. Each section is `y = c·(x − y₁) + x₁`, the first-order allpass
    /// `(c + z⁻¹)/(1 + c·z⁻¹)`.
    #[inline(always)]
    fn branches(&mut self, mut a: f32, mut b: f32) -> (f32, f32) {
        let mut i = 0;
        while i < N {
            let t = (a - self.y[i]) * self.coefs[i] + self.x[i];
            self.x[i] = a;
            self.y[i] = t;
            a = t;
            if i + 1 < N {
                let t = (b - self.y[i + 1]) * self.coefs[i + 1] + self.x[i + 1];
                self.x[i + 1] = b;
                self.y[i + 1] = t;
                b = t;
            }
            i += 2;
        }
        (a, b)
    }

    /// 2× upsample: one input sample in, two output samples out (in time
    /// order). Unity gain in the passband.
    #[inline]
    pub fn upsample(&mut self, input: f32) -> [f32; 2] {
        let (a, b) = self.branches(input, input);
        [a, b]
    }

    /// 2× downsample: two input samples (in time order) in, one out. Unity
    /// gain in the passband.
    #[inline]
    pub fn downsample(&mut self, input: [f32; 2]) -> f32 {
        let (a, b) = self.branches(input[1], input[0]);
        0.5 * (a + b)
    }
}

/// Oversampling factor for an [`Oversampler`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum OversampleFactor {
    Off = 0,
    X2 = 1,
    X4 = 2,
}

impl OversampleFactor {
    /// Display labels, indexed by discriminant.
    pub const LABELS: [&'static str; 3] = ["Off", "2x", "4x"];

    pub fn from_int(v: i32) -> Self {
        match v {
            1 => Self::X2,
            2 => Self::X4,
            _ => Self::Off,
        }
    }

    /// How many high-rate samples each base-rate sample becomes.
    pub fn ratio(self) -> usize {
        match self {
            Self::Off => 1,
            Self::X2 => 2,
            Self::X4 => 4,
        }
    }
}

/// Sections in the first (base ↔ 2×) stage. Its transition band sits right
/// under the base-rate Nyquist, so it carries the steep filter: 12 sections
/// at a 0.02 transition measure > 100 dB of stopband (see
/// `tests/oversample.rs`), with the passband flat to 0.46·fs_base.
const STAGE1_COEFS: usize = 12;
const STAGE1_TRANSITION: f64 = 0.02;

/// Sections in the second (2× ↔ 4×) stage. Only content that would fold
/// into the base band has to be rejected here — everything between the
/// base Nyquist and the 2× Nyquist is removed by stage 1 on the way down —
/// so the transition is wide and six sections keep a 4× round trip's
/// aliases more than 90 dB down (four measured only 86 dB).
const STAGE2_COEFS: usize = 6;
const STAGE2_TRANSITION: f64 = 0.12;

/// A mono 1×/2×/4× up/down-sampler pair.
///
/// Usage per base-rate sample:
///
/// ```
/// # use resonance_dsp::{Oversampler, OversampleFactor};
/// let mut os = Oversampler::new();
/// os.set_factor(OversampleFactor::X4);
/// let mut buf = os.upsample(0.5);
/// for s in &mut buf[..os.ratio()] {
///     *s = s.clamp(-0.25, 0.25); // the nonlinearity
/// }
/// let out = os.downsample(&buf);
/// # let _ = out;
/// ```
///
/// With [`OversampleFactor::Off`] both calls are exact pass-throughs.
///
/// **Mixing:** the up/down pair has a small, frequency-dependent phase
/// response. Blend dry and wet *inside* the high-rate loop (before
/// [`Oversampler::downsample`]) so both see the identical filter; mixing a
/// raw dry signal against the downsampled wet one comb-filters.
#[derive(Clone, Copy)]
pub struct Oversampler {
    factor: OversampleFactor,
    up1: Halfband<STAGE1_COEFS>,
    down1: Halfband<STAGE1_COEFS>,
    up2: Halfband<STAGE2_COEFS>,
    down2: Halfband<STAGE2_COEFS>,
}

impl Default for Oversampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Oversampler {
    /// Designs the filters (a handful of `f64` transcendentals) — call from
    /// `initialize`, not from the audio callback. Starts at
    /// [`OversampleFactor::Off`].
    pub fn new() -> Self {
        Self {
            factor: OversampleFactor::Off,
            up1: Halfband::new(STAGE1_TRANSITION),
            down1: Halfband::new(STAGE1_TRANSITION),
            up2: Halfband::new(STAGE2_TRANSITION),
            down2: Halfband::new(STAGE2_TRANSITION),
        }
    }

    pub fn factor(&self) -> OversampleFactor {
        self.factor
    }

    /// The number of valid high-rate samples [`Oversampler::upsample`]
    /// produces.
    pub fn ratio(&self) -> usize {
        self.factor.ratio()
    }

    /// Change the factor. Clears the filter state when it actually changes
    /// (the old state belongs to a different rate); a no-op otherwise, so it
    /// is safe to call once per block. RT-safe.
    pub fn set_factor(&mut self, factor: OversampleFactor) {
        if factor != self.factor {
            self.factor = factor;
            self.reset();
        }
    }

    pub fn reset(&mut self) {
        self.up1.reset();
        self.down1.reset();
        self.up2.reset();
        self.down2.reset();
    }

    /// One base-rate sample in; `ratio()` high-rate samples out, in the
    /// leading slots of the returned array (the rest are zero).
    #[inline]
    pub fn upsample(&mut self, input: f32) -> [f32; 4] {
        match self.factor {
            OversampleFactor::Off => [input, 0.0, 0.0, 0.0],
            OversampleFactor::X2 => {
                let [a, b] = self.up1.upsample(input);
                [a, b, 0.0, 0.0]
            }
            OversampleFactor::X4 => {
                let [a, b] = self.up1.upsample(input);
                let [a0, a1] = self.up2.upsample(a);
                let [b0, b1] = self.up2.upsample(b);
                [a0, a1, b0, b1]
            }
        }
    }

    /// `ratio()` high-rate samples in (the leading slots of `buf`); one
    /// base-rate sample out.
    #[inline]
    pub fn downsample(&mut self, buf: &[f32; 4]) -> f32 {
        match self.factor {
            OversampleFactor::Off => buf[0],
            OversampleFactor::X2 => self.down1.downsample([buf[0], buf[1]]),
            OversampleFactor::X4 => {
                let a = self.down2.downsample([buf[0], buf[1]]);
                let b = self.down2.downsample([buf[2], buf[3]]);
                self.down1.downsample([a, b])
            }
        }
    }
}
