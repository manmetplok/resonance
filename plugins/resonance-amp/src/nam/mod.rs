/// NAM (Neural Amp Modeler) model inference.
pub mod activations;
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

/// Validate buffer dimensions for a grouped matrix-vector operation. The
/// compact grouped weight holds `rows * cols / groups` values (each group is
/// a `[rows/groups x cols/groups]` row-major block); `groups == 1` is the
/// dense case of [`validate_matvec_dims`].
pub fn validate_grouped_matvec_dims(
    a: &[f32],
    x: &[f32],
    y: &[f32],
    rows: usize,
    cols: usize,
    groups: usize,
) -> bool {
    groups >= 1
        && rows.is_multiple_of(groups)
        && cols.is_multiple_of(groups)
        && a.len() >= rows * cols / groups
        && x.len() >= cols
        && y.len() >= rows
}

/// Grouped matrix-vector multiply: `y = blockdiag(A_0, ..., A_{G-1}) * x`.
///
/// Mirrors the reference NAM grouped `Conv1D`/`Conv1x1` semantics
/// (NAM/conv1d.cpp, NAM/dsp.cpp): the `cols` input channels and `rows`
/// output channels are split into `groups` contiguous blocks, and output
/// block g sees only input block g. `a` is the compact grouped weight —
/// `groups` concatenated row-major `[rows/groups x cols/groups]` matrices,
/// exactly the flat `[g][i][j]` order the reference consumes.
///
/// `groups == 1` dispatches to the dense [`matvec`] unchanged, so ungrouped
/// (A1) models keep the historical code path bit-for-bit.
#[inline(always)]
pub fn grouped_matvec(
    a: &[f32],
    x: &[f32],
    rows: usize,
    cols: usize,
    groups: usize,
    y: &mut [f32],
) {
    if groups <= 1 {
        matvec(a, x, rows, cols, y);
        return;
    }
    let opg = rows / groups;
    let ipg = cols / groups;
    let block = opg * ipg;
    for g in 0..groups {
        matvec(
            &a[g * block..(g + 1) * block],
            &x[g * ipg..(g + 1) * ipg],
            opg,
            ipg,
            &mut y[g * opg..(g + 1) * opg],
        );
    }
}

/// Grouped matrix-vector multiply-add: `y += blockdiag(A_0, ..., A_{G-1}) * x`.
///
/// Same layout and grouping semantics as [`grouped_matvec`]; `groups == 1`
/// dispatches to the dense [`matvec_add`] unchanged.
#[inline(always)]
pub fn grouped_matvec_add(
    a: &[f32],
    x: &[f32],
    rows: usize,
    cols: usize,
    groups: usize,
    y: &mut [f32],
) {
    if groups <= 1 {
        matvec_add(a, x, rows, cols, y);
        return;
    }
    let opg = rows / groups;
    let ipg = cols / groups;
    let block = opg * ipg;
    for g in 0..groups {
        matvec_add(
            &a[g * block..(g + 1) * block],
            &x[g * ipg..(g + 1) * ipg],
            opg,
            ipg,
            &mut y[g * opg..(g + 1) * opg],
        );
    }
}

/// Fast tanh — the EXACT rational approximation of the
/// NeuralAmpModelerCore reference (`nam::activations::fast_tanh`,
/// NAM/activations.h), term for term with the reference's own float
/// literals. The official NAM plugin runs with `enable_fast_tanh()`, so
/// A1-flavor files must use this precise formula for correct-vs-plugin
/// output (ba todo #1116; the engine's previous Padé-approximant fast tanh
/// deviated from the plugin by up to ~4e-3 at signal level).
#[allow(clippy::excessive_precision)]
#[inline(always)]
pub fn fast_tanh(x: f32) -> f32 {
    let ax = x.abs();
    let x2 = x * x;
    (x * (2.45550750702956f32 + 2.45550750702956f32 * ax
        + (0.893229853513558f32 + 0.821226666969744f32 * ax) * x2))
        / (2.44506634652299f32
            + (2.44506634652299f32 + x2) * (x + 0.814642734961073f32 * x * ax).abs())
}

/// Fast sigmoid derived from fast_tanh, in the reference's exact form
/// (`fast_sigmoid` in NAM/activations.h): 0.5 * (fast_tanh(x * 0.5) + 1).
#[inline(always)]
pub fn sigmoid(x: f32) -> f32 {
    0.5 * (fast_tanh(x * 0.5) + 1.0)
}
