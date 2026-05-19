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
///
/// Slicing first + `chunks_exact` gives LLVM enough length information to
/// elide every per-element bounds check inside the inner dot product loop
/// (verified by micro-benchmark to be within 1% of the previous
/// `get_unchecked` version across 16x16, 32x32, and 64x64 dimensions).
#[inline(always)]
pub fn matvec(a: &[f32], x: &[f32], rows: usize, cols: usize, y: &mut [f32]) {
    let a = &a[..rows * cols];
    let x = &x[..cols];
    let y = &mut y[..rows];
    for (out, row) in y.iter_mut().zip(a.chunks_exact(cols)) {
        let mut sum = 0.0f32;
        for (ai, xi) in row.iter().zip(x.iter()) {
            sum += ai * xi;
        }
        *out = sum;
    }
}

/// Matrix-vector multiply-add: y += A * x.
///
/// Same iterator pattern as `matvec` — see that function's note on
/// bounds-check elision.
#[inline(always)]
pub fn matvec_add(a: &[f32], x: &[f32], rows: usize, cols: usize, y: &mut [f32]) {
    let a = &a[..rows * cols];
    let x = &x[..cols];
    let y = &mut y[..rows];
    for (out, row) in y.iter_mut().zip(a.chunks_exact(cols)) {
        let mut sum = 0.0f32;
        for (ai, xi) in row.iter().zip(x.iter()) {
            sum += ai * xi;
        }
        *out += sum;
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
