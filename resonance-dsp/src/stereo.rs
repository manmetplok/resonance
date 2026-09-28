//! Mid/side and stereo-image helpers.
//!
//! The M/S convention is the halved one: `M = (L + R)/2`, `S = (L − R)/2`,
//! decoded by `L = M + S`, `R = M − S`, so a round trip is the identity
//! and `M` of a centred mono source equals the source.

use crate::stereo_balance;

/// `(L, R) → (M, S)` with `M = (L+R)/2`, `S = (L−R)/2`.
#[inline]
pub fn ms_encode(l: f32, r: f32) -> (f32, f32) {
    (0.5 * (l + r), 0.5 * (l - r))
}

/// `(M, S) → (L, R)` with `L = M + S`, `R = M − S`.
#[inline]
pub fn ms_decode(m: f32, s: f32) -> (f32, f32) {
    (m + s, m - s)
}

/// Scale the side signal by `width` (0 = mono, 1 = unchanged, 2 = double
/// side). The mid, and so the mono sum, is untouched. `width` 1 returns
/// the input exactly.
#[inline]
pub fn apply_width(l: f32, r: f32, width: f32) -> (f32, f32) {
    if width == 1.0 {
        return (l, r);
    }
    let (m, s) = ms_encode(l, r);
    ms_decode(m, s * width)
}

/// Stereo balance (not pan): `balance` −1..=1, centre is unity on both
/// sides and the far side fades linearly (see [`crate::stereo_balance`]).
#[inline]
pub fn apply_balance(l: f32, r: f32, balance: f32) -> (f32, f32) {
    let (gl, gr) = stereo_balance(balance);
    (l * gl, r * gr)
}

/// Stereo rotation: turns the L/R vector by an angle, which moves the
/// whole image sideways while preserving its energy (a goniometer shows
/// the trace rotating). Positive angles move the image toward the right;
/// π/4 puts a centred source hard right.
#[derive(Clone, Copy, Debug)]
pub struct StereoRotation {
    cos: f32,
    sin: f32,
}

impl Default for StereoRotation {
    fn default() -> Self {
        Self { cos: 1.0, sin: 0.0 }
    }
}

impl StereoRotation {
    /// A rotation by `radians`.
    pub fn new(radians: f32) -> Self {
        let mut r = Self::default();
        r.set_angle(radians);
        r
    }

    /// Set the angle, clamped to ±π/2. 0 is an exact passthrough.
    pub fn set_angle(&mut self, radians: f32) {
        let a = if radians.is_finite() {
            radians.clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2)
        } else {
            0.0
        };
        if a == 0.0 {
            *self = Self::default();
        } else {
            let (s, c) = a.sin_cos();
            self.sin = s;
            self.cos = c;
        }
    }

    #[inline]
    pub fn process(&self, l: f32, r: f32) -> (f32, f32) {
        if self.sin == 0.0 {
            return (l, r);
        }
        (l * self.cos - r * self.sin, l * self.sin + r * self.cos)
    }
}
