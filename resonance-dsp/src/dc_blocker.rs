/// One-pole DC-blocking high-pass: `y[n] = x[n] - x[n-1] + R*y[n-1]`.
///
/// With `R = 0.995` the -3 dB cutoff lands near 20 Hz at 44.1 kHz and near
/// 22 Hz at 48 kHz — inaudible, but enough to strip the static DC bias
/// that asymmetric waveshapers and NAM profiles introduce.
#[derive(Default, Clone, Copy)]
pub struct DcBlocker {
    x1: f32,
    y1: f32,
}

impl DcBlocker {
    pub const R: f32 = 0.995;

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }

    #[inline(always)]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + Self::R * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}
