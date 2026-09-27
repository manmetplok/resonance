/// NAM (Neural Amp Modeler) model inference.
pub mod activations;
pub(crate) mod gemm;
pub mod lstm;
pub mod parse;
pub mod wavenet;

/// Trait for NAM model inference. All buffers are pre-allocated at construction
/// time so `process_sample` is allocation-free.
pub trait NamInference: Send {
    fn process_sample(&mut self, input: f32) -> f32;

    /// Process a block: `output[i]` is what `process_sample(input[i])`
    /// would return, in order. Stream-equivalent to the per-sample call
    /// (up to f32 accumulation order) for any block split. The default
    /// runs sample by sample; the WaveNet overrides it with a block
    /// forward pass whose matrix products amortise across frames.
    fn process_block(&mut self, input: &[f32], output: &mut [f32]) {
        for (o, &x) in output.iter_mut().zip(input) {
            *o = self.process_sample(x);
        }
    }

    fn reset(&mut self);
}

/// Validate that buffer dimensions are consistent for matrix-vector operations.
/// Returns `true` if `a.len() >= rows * cols && x.len() >= cols && y.len() >= rows`.
pub fn validate_matvec_dims(a: &[f32], x: &[f32], y: &[f32], rows: usize, cols: usize) -> bool {
    a.len() >= rows * cols && x.len() >= cols && y.len() >= rows
}

/// Number of independent partial sums the dot product accumulates into.
///
/// A single running `sum += a * x` is a serial dependency chain: each FMA
/// waits ~4 cycles for the previous one, so a 16-tap dot product costs ~64
/// cycles no matter how wide the machine is. Rust's float semantics are
/// strict, so LLVM may not split that chain on its own — it has to be
/// written as independent lanes.
///
/// Eight lanes is one 256-bit vector, and two of them fit a 512-bit
/// register, so this shape maps onto AVX2 and AVX-512 alike (and onto
/// NEON as two 128-bit halves). Measured faster than 4 or 16 lanes on the
/// 8-32 column shapes NAM actually issues.
const DOT_LANES: usize = 8;

/// Dot product with `DOT_LANES` independent accumulators and a pairwise
/// tree reduction.
///
/// # Numerics
///
/// Summing in lanes reassociates the additions, so the result can differ
/// from a strict left-to-right sum in the last ulp or two (~1e-7 relative
/// for these lengths). That is deliberate and it moves *toward* the
/// reference, not away: the C++ NeuralAmpModelerCore this engine mirrors
/// uses Eigen, which vectorises its matvec the same way. The reference
/// parity tests budget 1e-6 mixed abs/rel for exactly this accumulation
/// gap (see `tests/common/mod.rs`).
#[inline(always)]
fn dot(row: &[f32], x: &[f32]) -> f32 {
    debug_assert_eq!(row.len(), x.len());
    let n = row.len();
    let full = n / DOT_LANES * DOT_LANES;

    let mut acc = [0.0f32; DOT_LANES];
    // `chunks_exact` on both sides gives LLVM the constant trip count it
    // needs to emit one packed FMA per chunk with no bounds checks.
    for (r, xc) in row[..full]
        .chunks_exact(DOT_LANES)
        .zip(x[..full].chunks_exact(DOT_LANES))
    {
        for l in 0..DOT_LANES {
            acc[l] += r[l] * xc[l];
        }
    }

    // Pairwise tree reduction: log2(DOT_LANES) dependent steps instead of
    // DOT_LANES serial adds.
    let mut width = DOT_LANES / 2;
    while width > 0 {
        for l in 0..width {
            acc[l] += acc[l + width];
        }
        width /= 2;
    }
    let mut sum = acc[0];

    // Tail for column counts that are not a multiple of DOT_LANES.
    for (a, b) in row[full..].iter().zip(&x[full..]) {
        sum += a * b;
    }
    sum
}

// Note for future tuning: driving 4 rows at once through a
// `[[f32; DOT_LANES]; 4]` accumulator bank — to give the out-of-order
// engine four independent reduction chains — was tried and measured
// *slower* by 23-32% across all four NAM shapes. LLVM spills the bank to
// the stack instead of keeping it in vector registers. Row-at-a-time with
// a single bank is the faster shape here.

/// Matrix-vector multiply: y = A * x, where A is [rows x cols] row-major.
///
/// See [`dot`] for the accumulation strategy and its numerical
/// consequences.
#[inline(always)]
pub fn matvec(a: &[f32], x: &[f32], rows: usize, cols: usize, y: &mut [f32]) {
    let a = &a[..rows * cols];
    let x = &x[..cols];
    let y = &mut y[..rows];
    for (out, row) in y.iter_mut().zip(a.chunks_exact(cols)) {
        *out = dot(row, x);
    }
}

/// Matrix-vector multiply-add: y += A * x.
///
/// Same kernel as [`matvec`], accumulating into `y` instead of
/// overwriting it.
#[inline(always)]
pub fn matvec_add(a: &[f32], x: &[f32], rows: usize, cols: usize, y: &mut [f32]) {
    let a = &a[..rows * cols];
    let x = &x[..cols];
    let y = &mut y[..rows];
    for (out, row) in y.iter_mut().zip(a.chunks_exact(cols)) {
        *out += dot(row, x);
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
