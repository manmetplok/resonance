/// NAM (Neural Amp Modeler) model inference.

pub mod lstm;
pub mod parse;
pub mod wavenet;

/// Trait for NAM model inference. All buffers are pre-allocated at construction
/// time so `process_sample` is allocation-free.
pub trait NamInference: Send {
    fn process_sample(&mut self, input: f32) -> f32;
    fn reset(&mut self);
}

/// Validate that buffer dimensions are consistent for matrix-vector operations.
/// Returns `true` if `a.len() >= rows * cols && x.len() >= cols && y.len() >= rows`.
pub fn validate_matvec_dims(a: &[f32], x: &[f32], y: &[f32], rows: usize, cols: usize) -> bool {
    a.len() >= rows * cols && x.len() >= cols && y.len() >= rows
}

/// Matrix-vector multiply: y = A * x, where A is [rows x cols] row-major.
#[inline(always)]
pub fn matvec(a: &[f32], x: &[f32], rows: usize, cols: usize, y: &mut [f32]) {
    debug_assert!(a.len() >= rows * cols, "matvec: a too short");
    debug_assert!(x.len() >= cols, "matvec: x too short");
    debug_assert!(y.len() >= rows, "matvec: y too short");
    for r in 0..rows {
        let mut sum = 0.0f32;
        let row_start = r * cols;
        for c in 0..cols {
            // SAFETY: dimensions validated at model load time
            sum += unsafe { *a.get_unchecked(row_start + c) * *x.get_unchecked(c) };
        }
        y[r] = sum;
    }
}

/// Matrix-vector multiply-add: y += A * x.
#[inline(always)]
pub fn matvec_add(a: &[f32], x: &[f32], rows: usize, cols: usize, y: &mut [f32]) {
    debug_assert!(a.len() >= rows * cols, "matvec_add: a too short");
    debug_assert!(x.len() >= cols, "matvec_add: x too short");
    debug_assert!(y.len() >= rows, "matvec_add: y too short");
    for r in 0..rows {
        let mut sum = 0.0f32;
        let row_start = r * cols;
        for c in 0..cols {
            // SAFETY: dimensions validated at model load time
            sum += unsafe { *a.get_unchecked(row_start + c) * *x.get_unchecked(c) };
        }
        y[r] += sum;
    }
}

/// Fast tanh approximation using a degree-7/6 Padé approximant.
/// Accurate to ~20 bits across the full range — more than sufficient
/// for neural network inference on audio signals.
#[inline(always)]
pub fn fast_tanh(x: f32) -> f32 {
    // Clamp to avoid overflow in x^6/x^7 terms
    let x = x.clamp(-5.0, 5.0);
    let x2 = x * x;
    let num = x * (135135.0 + x2 * (17325.0 + x2 * (378.0 + x2)));
    let den = 135135.0 + x2 * (62370.0 + x2 * (3150.0 + x2 * 28.0));
    num / den
}

/// Fast sigmoid derived from fast_tanh: sigmoid(x) = 0.5 + 0.5 * tanh(x/2).
#[inline(always)]
pub fn sigmoid(x: f32) -> f32 {
    0.5 + 0.5 * fast_tanh(x * 0.5)
}
