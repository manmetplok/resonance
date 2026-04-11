//! Two-pass gating for BS.1770-4 integrated loudness.
//!
//! Given a slice of per-block mean-square energies, compute the
//! gated integrated loudness in LUFS:
//!
//! 1. Convert each block MS to a block loudness `L_j = -0.691 + 10 log10(z_j)`.
//! 2. **Absolute gate**: drop blocks with `L_j < -70 LUFS`.
//! 3. Compute the ungated reference loudness from the mean of the surviving
//!    blocks' mean-squares.
//! 4. **Relative gate**: drop blocks with `L_j < reference - 10 LU`.
//! 5. Return `-0.691 + 10 log10(mean MS of final surviving blocks)`.
//!
//! Pure functions over a slice so the integrated accumulator can defer the
//! work to readout time and never run it on the audio thread.

/// BS.1770-4 loudness offset constant.
pub const LOUDNESS_OFFSET: f64 = -0.691;
/// Absolute silence gate threshold in LUFS.
pub const ABSOLUTE_GATE_LUFS: f64 = -70.0;
/// Relative gate offset from the ungated reference (LU).
pub const RELATIVE_GATE_LU: f64 = -10.0;

/// Convert a block mean-square to a block loudness in LUFS, returning
/// `f64::NEG_INFINITY` for zero or negative inputs so they naturally fail
/// the absolute gate.
#[inline]
pub fn block_mean_square_to_lufs(ms: f64) -> f64 {
    if ms > 0.0 {
        LOUDNESS_OFFSET + 10.0 * ms.log10()
    } else {
        f64::NEG_INFINITY
    }
}

/// Run the BS.1770-4 two-pass gating over a slice of block mean-squares
/// and return the gated integrated loudness in LUFS.
///
/// Returns `f64::NEG_INFINITY` if no blocks survive the absolute gate
/// (i.e. the source is effectively silent).
pub fn gated_integrated_lufs(blocks: &[f64]) -> f64 {
    if blocks.is_empty() {
        return f64::NEG_INFINITY;
    }

    // Pass 1: absolute gate.
    let mut abs_sum = 0.0_f64;
    let mut abs_count: usize = 0;
    for &ms in blocks {
        if block_mean_square_to_lufs(ms) >= ABSOLUTE_GATE_LUFS {
            abs_sum += ms;
            abs_count += 1;
        }
    }
    if abs_count == 0 {
        return f64::NEG_INFINITY;
    }

    // Reference loudness (ungated mean of absolute-gated blocks).
    let abs_mean_ms = abs_sum / abs_count as f64;
    let reference_lufs = block_mean_square_to_lufs(abs_mean_ms);
    let relative_threshold = reference_lufs + RELATIVE_GATE_LU;

    // Pass 2: relative gate. Must *also* pass the absolute gate
    // (block_mean_square_to_lufs returns NEG_INFINITY on 0.0, so the
    // >= test below correctly rejects those too).
    let mut rel_sum = 0.0_f64;
    let mut rel_count: usize = 0;
    for &ms in blocks {
        let lufs = block_mean_square_to_lufs(ms);
        if lufs >= ABSOLUTE_GATE_LUFS && lufs >= relative_threshold {
            rel_sum += ms;
            rel_count += 1;
        }
    }
    if rel_count == 0 {
        return f64::NEG_INFINITY;
    }

    block_mean_square_to_lufs(rel_sum / rel_count as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Convert a target LUFS value to its underlying mean-square energy.
    fn lufs_to_ms(lufs: f64) -> f64 {
        10.0_f64.powf((lufs - LOUDNESS_OFFSET) / 10.0)
    }

    #[test]
    fn single_level_signal_reports_that_level() {
        let blocks = vec![lufs_to_ms(-23.0); 200];
        let got = gated_integrated_lufs(&blocks);
        assert!((got - -23.0).abs() < 1e-6, "got {got}");
    }

    #[test]
    fn absolute_gate_drops_sub_70_lufs_blocks() {
        // 100 blocks at -23 LUFS, 100 at -80 LUFS (well below -70).
        let mut blocks = vec![lufs_to_ms(-23.0); 100];
        blocks.extend(vec![lufs_to_ms(-80.0); 100]);
        let got = gated_integrated_lufs(&blocks);
        assert!((got - -23.0).abs() < 1e-6, "got {got}");
    }

    #[test]
    fn relative_gate_drops_quiet_blocks_below_reference_minus_10() {
        // Half at -23, half at -40. The -40 blocks are more than 10 LU below
        // the ungated reference and should be dropped by the relative gate.
        let mut blocks = vec![lufs_to_ms(-23.0); 100];
        blocks.extend(vec![lufs_to_ms(-40.0); 100]);
        let got = gated_integrated_lufs(&blocks);
        assert!((got - -23.0).abs() < 0.1, "got {got}");
    }

    #[test]
    fn silent_input_returns_neg_infinity() {
        let blocks = vec![0.0; 100];
        let got = gated_integrated_lufs(&blocks);
        assert!(got.is_infinite() && got < 0.0);
    }

    #[test]
    fn empty_input_returns_neg_infinity() {
        let got = gated_integrated_lufs(&[]);
        assert!(got.is_infinite() && got < 0.0);
    }
}
