//! Fractional-delay interpolation primitives.
//!
//! Granular resampling reads a circular buffer at non-integer positions;
//! linear interpolation is audibly dull and aliased on transposed grains,
//! so the shared primitive here is 4-point cubic Hermite (Catmull-Rom)
//! interpolation — the standard sampler compromise between cost and image
//! rejection (research doc #252 §3). Both functions are pure, allocation-
//! free and lock-free, safe for per-sample audio-thread use.

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
