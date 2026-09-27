//! Block matrix products for the WaveNet block forward pass.
//!
//! The per-sample engine evaluated every conv as a matrix-VECTOR product,
//! and each output row of a matvec ends in a horizontal reduction — the
//! 16x16 shape the stock "standard" capture issues ran at ~9 GMAC/s. The
//! block pass instead multiplies a whole block of frames at once
//! (`Y[n x cout] += X[n x cin] * W^T`), which has no reductions at all:
//! each input value is broadcast against a row of the TRANSPOSED weight
//! (`wt[i][o]`) and accumulated into a register tile of `FRAMES x 8`
//! outputs.
//! This is the shape Eigen's GEMM uses in the C++ reference.
//!
//! # Numerics
//!
//! Each call sums its `cin` products from zero in order `i = 0..cin` and
//! adds the result to `y` (`y += W x`, the grouping of the per-sample
//! `matvec_add`). For `cin < 8` that is the old per-sample sum exactly;
//! wider inputs differ from the lane-split [`super::matvec`] in the last
//! ulp or two — accumulation-order noise the reference parity budgets
//! already cover. Multiply and add stay separate (Rust never contracts
//! them, and an explicit `mul_add` measured further from the reference
//! renders: the slimmable fixture went from bit-identical to 2.3e-6).

/// Frames per register tile: 8 frames x 8 lanes is 8 accumulator vectors
/// plus the weight row and broadcasts, inside AVX2's 16 registers.
/// Measured on the standard capture: 2 -> 64 us, 4 -> 55 us, 8 -> 52 us.
const FRAMES: usize = 8;
/// Output lanes per chunk: one 256-bit vector of f32.
const LANES: usize = 8;

/// Strided view of a block of frames: frame `t`, channel `i` lives at
/// `data[t * stride + i]`.
#[derive(Clone, Copy)]
pub(crate) struct Frames<'a> {
    pub data: &'a [f32],
    pub stride: usize,
}

/// Row stride of a transposed weight for `cout` outputs: padded up to a
/// whole number of vector chunks, so narrow layers (the lite / feather /
/// nano architectures run 2-12 channels) still go through the vector tile.
#[inline(always)]
pub(crate) fn padded(cout: usize) -> usize {
    cout.next_multiple_of(LANES)
}

/// `y[t*ys + o] += sum_i x[t*xs + i] * wt[i*padded(cout) + o]` for
/// `t < n`, `o < cout`, `i < cin` — a dense block product against a
/// transposed, lane-padded weight (see [`transpose_grouped`]).
#[inline]
pub(crate) fn gemm_acc(
    x: Frames<'_>,
    wt: &[f32],
    cin: usize,
    cout: usize,
    y: &mut [f32],
    ys: usize,
    n: usize,
) {
    if n == 0 || cout == 0 {
        return;
    }
    let cpad = padded(cout);
    debug_assert!(wt.len() >= cin * cpad);
    debug_assert!(x.data.len() >= (n - 1) * x.stride + cin);
    debug_assert!(y.len() >= (n - 1) * ys + cout);

    let full = cout / LANES * LANES;
    let rest = cout - full;
    let mut t = 0;
    while t + FRAMES <= n {
        let mut o = 0;
        while o < full {
            tile::<FRAMES, true>(x, wt, cin, cpad, y, ys, t, o, LANES);
            o += LANES;
        }
        if rest > 0 {
            tile::<FRAMES, false>(x, wt, cin, cpad, y, ys, t, full, rest);
        }
        t += FRAMES;
    }
    // Leftover frames (n not a multiple of FRAMES, e.g. single-sample
    // calls during a model crossfade).
    while t < n {
        let mut o = 0;
        while o < full {
            tile::<1, true>(x, wt, cin, cpad, y, ys, t, o, LANES);
            o += LANES;
        }
        if rest > 0 {
            tile::<1, false>(x, wt, cin, cpad, y, ys, t, full, rest);
        }
        t += 1;
    }
}

/// Grouped block product: `groups` independent `[cin/g -> cout/g]` blocks
/// over contiguous channel ranges (reference grouped `Conv1D`/`Conv1x1`).
/// `wt` holds `groups` concatenated transposed, lane-padded per-group
/// blocks (`[cin/g x padded(cout/g)]` each). `groups == 1` is
/// [`gemm_acc`].
#[inline]
#[allow(clippy::too_many_arguments)]
pub(crate) fn grouped_gemm_acc(
    x: Frames<'_>,
    wt: &[f32],
    cin: usize,
    cout: usize,
    groups: usize,
    y: &mut [f32],
    ys: usize,
    n: usize,
) {
    if groups <= 1 {
        gemm_acc(x, wt, cin, cout, y, ys, n);
        return;
    }
    let ipg = cin / groups;
    let opg = cout / groups;
    let block = ipg * padded(opg);
    for g in 0..groups {
        gemm_acc(
            Frames {
                data: &x.data[g * ipg..],
                stride: x.stride,
            },
            &wt[g * block..(g + 1) * block],
            ipg,
            opg,
            &mut y[g * opg..],
            ys,
            n,
        );
    }
}

/// One `F x LANES` register tile: frames `t0..t0+F`, outputs
/// `o0..o0+width`. `FULL` tiles have `width == LANES`; a partial tile
/// (a layer's last chunk) computes all lanes against the zero padding of
/// the weight and stores only `width` of them.
///
/// Sums from zero and adds to `y` at the end: `y += (W x)` per call, the
/// grouping the per-sample matvec_add used (and the reference's per-tap
/// `Conv1D` accumulation), rather than one running sum through every tap.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn tile<const F: usize, const FULL: bool>(
    x: Frames<'_>,
    wt: &[f32],
    cin: usize,
    cpad: usize,
    y: &mut [f32],
    ys: usize,
    t0: usize,
    o0: usize,
    width: usize,
) {
    let mut acc = [[0.0f32; LANES]; F];
    let rows: [&[f32]; F] = std::array::from_fn(|f| &x.data[(t0 + f) * x.stride..][..cin]);
    for i in 0..cin {
        let w: &[f32; LANES] = wt[i * cpad + o0..][..LANES].try_into().unwrap();
        for f in 0..F {
            let xv = rows[f][i];
            for l in 0..LANES {
                acc[f][l] += xv * w[l];
            }
        }
    }
    for (f, a) in acc.iter().enumerate() {
        let row = &mut y[(t0 + f) * ys + o0..];
        if FULL {
            for (yo, v) in row[..LANES].iter_mut().zip(a) {
                *yo += v;
            }
        } else {
            for (yo, v) in row[..width].iter_mut().zip(a) {
                *yo += v;
            }
        }
    }
}

/// Transpose a compact grouped weight for the block GEMM: `groups`
/// row-major `[rows/g x cols/g]` blocks become `[cols/g x padded(rows/g)]`
/// blocks, zero in the padding lanes — the layout [`grouped_gemm_acc`]
/// consumes. Load-time only.
pub(crate) fn transpose_grouped(w: &[f32], rows: usize, cols: usize, groups: usize) -> Vec<f32> {
    let g = groups.max(1);
    let (r, c) = (rows / g, cols / g);
    let rpad = padded(r);
    let mut out = vec![0.0f32; g * c * rpad];
    for (blk, dst) in w.chunks_exact(r * c).take(g).zip(out.chunks_exact_mut(c * rpad)) {
        for i in 0..r {
            for j in 0..c {
                dst[j * rpad + i] = blk[i * c + j];
            }
        }
    }
    out
}
