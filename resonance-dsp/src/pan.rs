/// Constant-power pan law.
///
/// Takes a pan value in -1.0 (hard left) to 1.0 (hard right)
/// and returns (left_gain, right_gain) using equal-power panning.
#[inline]
pub fn constant_power_pan(pan: f32) -> (f32, f32) {
    let pan = pan.clamp(-1.0, 1.0);
    let angle = (pan + 1.0) * 0.25 * std::f32::consts::PI;
    (angle.cos(), angle.sin())
}

/// Stereo balance control.
///
/// Unlike [`constant_power_pan`], this does **not** attenuate at centre
/// (returns `(1.0, 1.0)` when `pan == 0.0`). Use this when the signal
/// is already stereo and a track-level constant-power pan will be
/// applied downstream.
#[inline]
pub fn stereo_balance(pan: f32) -> (f32, f32) {
    let pan = pan.clamp(-1.0, 1.0);
    let l = if pan <= 0.0 { 1.0 } else { 1.0 - pan };
    let r = if pan >= 0.0 { 1.0 } else { 1.0 + pan };
    (l, r)
}
