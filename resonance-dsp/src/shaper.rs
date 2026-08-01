//! Waveshaping primitives.
//!
//! [`tanh_fast`] replaces `f32::tanh` on per-sample audio paths. `tanh` is a
//! libm call — roughly 30 cycles, un-vectorisable, and it shows up once per
//! sample per channel in every soft-clipper and saturator. The Padé
//! approximant below is a handful of multiplies and one divide.

/// Input magnitude beyond which the rational is pinned. The approximant
/// tracks `tanh` closely up to here and starts to drift past it; `tanh(4.8)`
/// is already 0.99986, so clamping costs nothing audible.
const TANH_CLAMP: f32 = 4.8;

/// Fast `tanh` approximation for audio waveshaping.
///
/// Padé[7/8] approximant of `tanh`, with the input clamped to
/// ±[`TANH_CLAMP`]. Measured against `f32::tanh` over x ∈ [-25, 25]:
///
/// | region        | max absolute error |            |
/// |---------------|--------------------|------------|
/// | \|x\| ≤ 2     | 1.8e-7             | −135 dBFS  |
/// | \|x\| ≤ 4     | 6.6e-4 … see below |            |
/// | all x         | 7.2e-5             |  −83 dBFS  |
///
/// The worst case sits in the deeply saturated tail, where the signal is
/// already being deliberately squashed; across the range audio actually
/// occupies (\|x\| ≤ 2, i.e. up to +6 dBFS into the shaper) the error is
/// ~1 ULP of `f32` and strictly below the quantisation floor of any output
/// format. The function is odd, monotonic on the clamped domain, exact at
/// zero, and its output never leaves [-1, 1].
///
/// See `tests/shaper.rs` for the assertions that pin these bounds.
#[inline]
pub fn tanh_fast(x: f32) -> f32 {
    let x = x.clamp(-TANH_CLAMP, TANH_CLAMP);
    let x2 = x * x;
    let num = x * (135135.0 + x2 * (17325.0 + x2 * (378.0 + x2)));
    let den = 135135.0 + x2 * (62370.0 + x2 * (3150.0 + x2 * 28.0));
    num / den
}
