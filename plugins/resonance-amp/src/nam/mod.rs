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

/// Matrix-vector multiply: y = A * x, where A is [rows x cols] row-major.
#[inline]
pub fn matvec(a: &[f32], x: &[f32], rows: usize, cols: usize, y: &mut [f32]) {
    for r in 0..rows {
        let mut sum = 0.0f32;
        let row_start = r * cols;
        for c in 0..cols {
            sum += unsafe { *a.get_unchecked(row_start + c) * *x.get_unchecked(c) };
        }
        y[r] = sum;
    }
}

/// Matrix-vector multiply-add: y += A * x.
#[inline]
pub fn matvec_add(a: &[f32], x: &[f32], rows: usize, cols: usize, y: &mut [f32]) {
    for r in 0..rows {
        let mut sum = 0.0f32;
        let row_start = r * cols;
        for c in 0..cols {
            sum += unsafe { *a.get_unchecked(row_start + c) * *x.get_unchecked(c) };
        }
        y[r] += sum;
    }
}

#[inline(always)]
pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}
