/// Constant-power pan law.
///
/// Takes a pan value in -1.0 (hard left) to 1.0 (hard right)
/// and returns (left_gain, right_gain) using equal-power panning.
#[inline]
pub fn constant_power_pan(pan: f32) -> (f32, f32) {
    let angle = (pan + 1.0) * 0.25 * std::f32::consts::PI;
    (angle.cos(), angle.sin())
}
