//! Fractional-delay interpolation primitives.
//!
//! Granular resampling reads a circular buffer at non-integer positions;
//! linear interpolation is audibly dull and aliased on transposed grains,
//! so the shared default here is 4-point cubic Hermite (Catmull-Rom)
//! interpolation — the standard sampler compromise between cost and image
//! rejection (research doc #252 §3). The quality tiers (ba todo #1083)
//! add the two ends of the trade-off: 2-point linear for the Lo-fi tier
//! and a 6-point, 5th-order B-spline for HQ. All functions are pure,
//! allocation-free and lock-free, safe for per-sample audio-thread use.

/// 4-point cubic Hermite (Catmull-Rom) interpolation.
///
/// Interpolates between `x0` (at `frac = 0`) and `x1` (at `frac = 1`)
/// using the outer neighbours `xm1` and `x2` to estimate the endpoint
/// slopes via central differences. Exactly reproduces polynomials up to
/// degree 2 sampled on a uniform grid (central differences are exact for
/// quadratics), and in particular any linear ramp.
#[inline]
pub fn hermite4(xm1: f32, x0: f32, x1: f32, x2: f32, frac: f32) -> f32 {
    // Horner-form Catmull-Rom: c0 + f·(c1 + f·(c2 + f·c3)).
    let c0 = x0;
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * frac + c2) * frac + c1) * frac + c0
}

/// 6-point, 5th-order uniform B-spline interpolation (ba todo #1083).
///
/// Evaluates the quintic uniform B-spline through the six neighbours
/// `xm2..x3` at `frac` (`0` = at `x0`, `1` = at `x1`). Chosen for the
/// HQ tier over 2× oversampled Hermite reads because it needs no extra
/// buffer or resampling pass and, per Niemitalo's "Polynomial
/// Interpolators for High-Quality Resampling of Oversampled Audio"
/// (deip.pdf; doc #252 §3), the B-spline family has by far the best
/// stopband (image rejection) of the equal-cost polynomial
/// interpolators — exactly what transposed grain reads need, since the
/// audible artifact of resampling is the folded image energy. The
/// trade-off is mild passband droop (it is an approximating, not an
/// interpolating, kernel), inaudible on grain clouds and much cheaper
/// than the windowed-sinc alternative.
///
/// The kernel is a partition of unity and reproduces constants and
/// linear ramps exactly (quintic B-splines reproduce degree-1
/// polynomials); both properties are locked in by tests.
#[inline]
pub fn bspline6(xm2: f32, xm1: f32, x0: f32, x1: f32, x2: f32, x3: f32, frac: f32) -> f32 {
    // Blending functions of the uniform quintic B-spline in Horner
    // form, scaled by 1/120 once at the end.
    let u = frac;
    let b0 = ((((-u + 5.0) * u - 10.0) * u + 10.0) * u - 5.0) * u + 1.0; // (1-u)^5
    let b1 = (((5.0 * u - 20.0) * u + 20.0) * u + 20.0) * u * u - 50.0 * u + 26.0;
    let b2 = ((-10.0 * u + 30.0) * u * u - 60.0) * u * u + 66.0;
    let b3 = (((10.0 * u - 20.0) * u - 20.0) * u + 20.0) * u * u + 50.0 * u + 26.0;
    let b4 = ((((-5.0 * u + 5.0) * u + 10.0) * u + 10.0) * u + 5.0) * u + 1.0;
    let b5 = u * u * u * u * u;
    (xm2 * b0 + xm1 * b1 + x0 * b2 + x1 * b3 + x2 * b4 + x3 * b5) * (1.0 / 120.0)
}

/// Read a power-of-two circular buffer at a fractional `index` using
/// 2-point linear interpolation (Lo-fi tier, ba todo #1083; doc #252
/// §3: audibly duller and worse image rejection than Hermite — the
/// point of the tier).
///
/// Same wrapping contract as [`read_hermite_wrapped`].
///
/// # Panics
/// Panics if `buffer.len()` is not a power of two or is smaller than 4.
#[inline]
pub fn read_linear_wrapped(buffer: &[f32], index: f64) -> f32 {
    let len = buffer.len();
    assert!(
        len >= 4 && len.is_power_of_two(),
        "buffer length must be a power of two >= 4, got {len}"
    );
    let mask = (len - 1) as i64;
    let i0 = index.floor();
    let frac = (index - i0) as f32;
    let i0 = i0 as i64;
    let x0 = buffer[(i0 & mask) as usize];
    let x1 = buffer[((i0 + 1) & mask) as usize];
    x0 + (x1 - x0) * frac
}

/// Read a power-of-two circular buffer at a fractional `index` using
/// [`bspline6`] (HQ tier, ba todo #1083).
///
/// Same wrapping contract as [`read_hermite_wrapped`].
///
/// # Panics
/// Panics if `buffer.len()` is not a power of two or is smaller than 8
/// (the kernel spans six samples).
#[inline]
pub fn read_bspline6_wrapped(buffer: &[f32], index: f64) -> f32 {
    let len = buffer.len();
    assert!(
        len >= 8 && len.is_power_of_two(),
        "buffer length must be a power of two >= 8, got {len}"
    );
    let mask = (len - 1) as i64;
    let i0 = index.floor();
    let frac = (index - i0) as f32;
    let i0 = i0 as i64;
    let xm2 = buffer[((i0 - 2) & mask) as usize];
    let xm1 = buffer[((i0 - 1) & mask) as usize];
    let x0 = buffer[(i0 & mask) as usize];
    let x1 = buffer[((i0 + 1) & mask) as usize];
    let x2 = buffer[((i0 + 2) & mask) as usize];
    let x3 = buffer[((i0 + 3) & mask) as usize];
    bspline6(xm2, xm1, x0, x1, x2, x3, frac)
}

/// Read a power-of-two circular buffer at a fractional `index` using
/// [`hermite4`].
///
/// `buffer.len()` must be a power of two (≥ 4); indices wrap with a
/// bitmask, so any finite `index` — including negative values — is valid
/// and reads modulo the buffer length. The index is `f64` so long-running
/// phase accumulators keep sub-sample precision over hours of audio
/// (doc #252 §3).
///
/// # Panics
/// Panics if `buffer.len()` is not a power of two or is smaller than 4.
#[inline]
pub fn read_hermite_wrapped(buffer: &[f32], index: f64) -> f32 {
    let len = buffer.len();
    assert!(
        len >= 4 && len.is_power_of_two(),
        "buffer length must be a power of two >= 4, got {len}"
    );
    let mask = (len - 1) as i64;
    let i0 = index.floor();
    let frac = (index - i0) as f32;
    // A power-of-two mask on two's-complement integers is a true modulo,
    // so negative indices wrap to the end of the buffer.
    let i0 = i0 as i64;
    let xm1 = buffer[((i0 - 1) & mask) as usize];
    let x0 = buffer[(i0 & mask) as usize];
    let x1 = buffer[((i0 + 1) & mask) as usize];
    let x2 = buffer[((i0 + 2) & mask) as usize];
    hermite4(xm1, x0, x1, x2, frac)
}
