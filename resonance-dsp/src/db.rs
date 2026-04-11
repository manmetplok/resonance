/// Logarithmic / linear amplitude conversion helpers shared across plugins.

/// Floor used by `linear_to_db` for values at or below ~-200 dB. Chosen so that
/// downstream code can detect "silence" without needing `-inf` handling.
pub const MIN_DB: f32 = -120.0;

/// Convert a linear amplitude to decibels. Values at or below ~1e-10 clamp to
/// [`MIN_DB`] so callers never see `-inf`.
#[inline]
pub fn linear_to_db(v: f32) -> f32 {
    if v <= 1e-10 {
        MIN_DB
    } else {
        20.0 * v.log10()
    }
}

/// Convert decibels to linear amplitude.
#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}
