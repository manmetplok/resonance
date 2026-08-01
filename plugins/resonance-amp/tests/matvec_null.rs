//! Null test for the vectorised NAM matvec kernel.
//!
//! `matvec` accumulates into `DOT_LANES` independent partial sums and
//! reduces them pairwise, which reassociates the float additions relative
//! to a strict left-to-right sum. This pins the resulting deviation: it
//! must stay at rounding level for every shape the WaveNet forward pass
//! issues, so the change is inaudible and stays well inside the 1e-6
//! budget the reference-parity suite allows.
//!
//! The stronger correctness evidence is `nam_a1_reference_parity.rs` /
//! `nam_a2_reference_parity.rs`, which compare full model output against
//! captured C++ NeuralAmpModelerCore reference runs. This test isolates
//! the kernel so a regression points straight at the arithmetic.

use resonance_amp::nam::{matvec, matvec_add};

/// Strict left-to-right dot product — the accumulation order the kernel
/// used before it was vectorised.
fn reference_matvec(a: &[f32], x: &[f32], rows: usize, cols: usize, y: &mut [f32]) {
    for r in 0..rows {
        let mut sum = 0.0f32;
        for c in 0..cols {
            sum += a[r * cols + c] * x[c];
        }
        y[r] = sum;
    }
}

/// Deterministic pseudo-random weights in the range NAM weights actually
/// occupy (roughly ±1, mostly small).
fn weights(n: usize, seed: u64) -> Vec<f32> {
    let mut state = seed | 1;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            ((state >> 40) as f32 / 8_388_608.0) - 1.0
        })
        .collect()
}

/// Every (rows, cols) shape the standard and A2 WaveNet architectures
/// issue, plus non-multiples of the lane width to exercise the tail.
const SHAPES: &[(usize, usize)] = &[
    (32, 16),
    (16, 16),
    (8, 16),
    (16, 8),
    (64, 32),
    (32, 32),
    (12, 12),
    (7, 5),
    (1, 1),
    (3, 17),
    (16, 1),
];

#[test]
fn matvec_matches_strict_scalar_sum_within_rounding() {
    for &(rows, cols) in SHAPES {
        let a = weights(rows * cols, 0x9E37_79B9_7F4A_7C15 ^ (rows as u64) << 32 | cols as u64);
        let x = weights(cols, 0xD1B5_4A32_D192_ED03 ^ cols as u64);

        let mut got = vec![0.0f32; rows];
        let mut want = vec![0.0f32; rows];
        matvec(&a, &x, rows, cols, &mut got);
        reference_matvec(&a, &x, rows, cols, &mut want);

        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            // Mixed absolute/relative: the dot products are O(1) in
            // magnitude, so 1e-6 here is several orders tighter than the
            // 1e-6 end-to-end model tolerance.
            let err = (g - w).abs() / w.abs().max(1.0);
            assert!(
                err < 1e-6,
                "matvec {rows}x{cols} row {i}: got {g}, want {w}, err {err:.3e}"
            );
        }
    }
}

#[test]
fn matvec_add_accumulates_into_existing_values() {
    for &(rows, cols) in SHAPES {
        let a = weights(rows * cols, 0x2545_F491_4F6C_DD1D ^ rows as u64);
        let x = weights(cols, 0x1405_7B7E_F767_814F ^ cols as u64);
        let seed_y = weights(rows, 0xA076_1D64_78BD_642F);

        let mut got = seed_y.clone();
        matvec_add(&a, &x, rows, cols, &mut got);

        let mut product = vec![0.0f32; rows];
        reference_matvec(&a, &x, rows, cols, &mut product);

        for (i, ((g, p), s)) in got.iter().zip(&product).zip(&seed_y).enumerate() {
            let want = s + p;
            let err = (g - want).abs() / want.abs().max(1.0);
            assert!(
                err < 1e-6,
                "matvec_add {rows}x{cols} row {i}: got {g}, want {want}, err {err:.3e}"
            );
        }
    }
}

/// Buffers longer than `rows`/`cols` must be left untouched past the
/// declared extent — the forward pass relies on this, since its scratch
/// buffers are sized for the widest layer and reused by narrower ones.
#[test]
fn matvec_does_not_write_past_declared_rows() {
    let (rows, cols) = (8usize, 16usize);
    let a = weights(rows * cols, 0x51);
    let x = weights(cols + 8, 0x77);

    let mut y = vec![-1.0f32; rows + 8];
    matvec(&a, &x, rows, cols, &mut y);

    for (i, v) in y.iter().enumerate().skip(rows) {
        assert_eq!(*v, -1.0, "matvec wrote past row {rows} at index {i}");
    }
}
