//! Lossless (orthogonal) feedback matrices for [`super::Fdn`], applied in
//! place without storing the matrix.

/// Which orthogonal matrix mixes the lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatrixKind {
    /// `I − (2/N)·11ᵀ`: any N, O(N). Every line feeds every other line
    /// with the same weight `2/N`; for large N the diagonal `1 − 2/N`
    /// dominates, so it mixes slowly beyond about 8 lines.
    Householder,
    /// Normalised Sylvester Hadamard `H_N / √N` by the fast butterfly,
    /// O(N log N). N must be a power of two. Every entry is `±1/√N`:
    /// maximal mixing, the choice for 16 lines.
    Hadamard,
}

impl MatrixKind {
    /// Householder below 16 lines, Hadamard from 16 up when N is a power
    /// of two.
    pub const fn default_for(n: usize) -> Self {
        if n >= 16 && n.is_power_of_two() {
            Self::Hadamard
        } else {
            Self::Householder
        }
    }

    /// Apply the matrix to `x` in place.
    #[inline]
    pub fn apply<const N: usize>(self, x: &mut [f32; N]) {
        match self {
            Self::Householder => householder_in_place(x),
            Self::Hadamard => hadamard_in_place(x),
        }
    }
}

/// `x ← (I − (2/N)·11ᵀ)·x`.
#[inline]
pub fn householder_in_place<const N: usize>(x: &mut [f32; N]) {
    let k = x.iter().sum::<f32>() * (2.0 / N as f32);
    for v in x.iter_mut() {
        *v -= k;
    }
}

/// `x ← (H_N / √N)·x`, Sylvester ordering. Panics (debug) unless N is a
/// power of two.
#[inline]
pub fn hadamard_in_place<const N: usize>(x: &mut [f32; N]) {
    debug_assert!(N.is_power_of_two(), "Hadamard needs a power-of-two size, got {N}");
    let mut h = 1;
    while h < N {
        let mut i = 0;
        while i < N {
            for j in i..i + h {
                let (a, b) = (x[j], x[j + h]);
                x[j] = a + b;
                x[j + h] = a - b;
            }
            i += 2 * h;
        }
        h *= 2;
    }
    let scale = 1.0 / (N as f32).sqrt();
    for v in x.iter_mut() {
        *v *= scale;
    }
}
