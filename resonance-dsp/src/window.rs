//! Analysis/design window functions.

/// Fill `window` with a symmetric Hann window:
/// `w[i] = 0.5 − 0.5·cos(τ·i / (len − 1))`. Endpoints are zero; for
/// odd lengths the centre tap is exactly 1.
pub fn fill_hann_window(window: &mut [f32]) {
    let len = window.len();
    for (i, w) in window.iter_mut().enumerate() {
        let x = i as f32 / (len as f32 - 1.0);
        *w = 0.5 - 0.5 * (std::f32::consts::TAU * x).cos();
    }
}

/// Allocate a symmetric Hann window of `len` coefficients.
pub fn hann_window(len: usize) -> Vec<f32> {
    let mut window = vec![0.0; len];
    fill_hann_window(&mut window);
    window
}
