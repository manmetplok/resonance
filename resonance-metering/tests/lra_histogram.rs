//! Regression: the histogram-based `LraMeter` must agree with the exact
//! sort-based EBU 3342 computation it replaced (which allocated and
//! sorted on every readout — unacceptable on the audio thread, where
//! `lra_lu()` runs once per block via `ABMeterTap::snapshot`).
//!
//! The exact reference below is a faithful copy of the removed
//! implementation: absolute gate at -70 LUFS, relative gate at the
//! energetic mean of the absolute-gated set minus 20 LU, then the
//! linear-interpolated p95 - p10 of the sorted survivors.

use resonance_metering::lufs::gating::{block_mean_square_to_lufs, ABSOLUTE_GATE_LUFS};
use resonance_metering::lra::LRA_RELATIVE_GATE_LU;
use resonance_metering::LraMeter;

/// Worst-case deviation the 0.05 LU histogram binning may introduce: one
/// bucket per percentile.
const TOLERANCE_LU: f64 = 0.1;

fn lufs_to_ms(lufs: f64) -> f64 {
    10.0_f64.powf((lufs + 0.691) / 10.0)
}

/// The pre-histogram exact LRA: collect, gate, sort, interpolate.
fn exact_lra_lu(blocks: &[f64]) -> f64 {
    if blocks.is_empty() {
        return 0.0;
    }
    let mut abs_sum_ms = 0.0_f64;
    let mut abs_lufs: Vec<f64> = Vec::with_capacity(blocks.len());
    for &ms in blocks {
        let l = block_mean_square_to_lufs(ms);
        if l >= ABSOLUTE_GATE_LUFS {
            abs_sum_ms += ms;
            abs_lufs.push(l);
        }
    }
    if abs_lufs.is_empty() {
        return 0.0;
    }
    let reference_lufs = block_mean_square_to_lufs(abs_sum_ms / abs_lufs.len() as f64);
    let threshold = reference_lufs + LRA_RELATIVE_GATE_LU;

    let mut gated: Vec<f64> = abs_lufs.into_iter().filter(|&l| l >= threshold).collect();
    if gated.is_empty() {
        return 0.0;
    }
    gated.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    percentile(&gated, 0.95) - percentile(&gated, 0.10)
}

/// Linear-interpolated percentile of a sorted slice.
fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let pos = pct * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(sorted.len() - 1);
    let frac = pos - lo as f64;
    sorted[lo] + (sorted[hi] - sorted[lo]) * frac
}

fn assert_agreement(blocks: &[f64], label: &str) {
    let mut meter = LraMeter::new();
    for &ms in blocks {
        meter.push_short_term_mean_square(ms);
    }
    let exact = exact_lra_lu(blocks);
    let histogram = meter.lra_lu() as f64;
    assert!(
        (histogram - exact).abs() <= TOLERANCE_LU,
        "{label}: histogram LRA {histogram:.4} LU vs exact {exact:.4} LU \
         (deviation {:.4} > {TOLERANCE_LU} LU)",
        (histogram - exact).abs()
    );
}

#[test]
fn agrees_with_exact_on_synthetic_loudness_walk() {
    // A deterministic pseudo-random loudness walk (LCG) spanning the
    // range a real session covers, including sub-absolute-gate silence
    // and quiet passages that the -20 LU relative gate cuts.
    let mut state = 0x2545F491_u64;
    let mut next = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) as f64 / (1u64 << 31) as f64 // [0, 1)
    };
    let blocks: Vec<f64> = (0..1800)
        .map(|i| {
            let lufs = if i % 97 == 0 {
                -80.0 // silence, absolute-gated away
            } else if i % 13 == 0 {
                -55.0 - next() * 10.0 // quiet tail near the relative gate
            } else {
                -30.0 + next() * 16.0 // program material, -30..-14 LUFS
            };
            lufs_to_ms(lufs)
        })
        .collect();
    assert_agreement(&blocks, "synthetic walk");
}

#[test]
fn agrees_with_exact_on_discrete_step_sequence() {
    // Discrete levels land whole buckets on their exact mean loudness,
    // so the EBU 3342 step material should agree essentially exactly.
    let mut blocks = Vec::new();
    blocks.extend(std::iter::repeat_n(lufs_to_ms(-20.0), 20));
    blocks.extend(std::iter::repeat_n(lufs_to_ms(-30.0), 20));
    blocks.extend(std::iter::repeat_n(lufs_to_ms(-20.0), 20));
    assert_agreement(&blocks, "step sequence");
}

#[test]
fn agrees_with_exact_when_relative_gate_dominates() {
    // A handful of hot blocks drag the energetic-mean reference up far
    // enough that the -20 LU relative gate cuts the quiet majority.
    let mut blocks = Vec::new();
    blocks.extend(std::iter::repeat_n(lufs_to_ms(-5.0), 5));
    blocks.extend(std::iter::repeat_n(lufs_to_ms(-45.0), 60));
    assert_agreement(&blocks, "relative-gate dominated");
}

#[test]
fn reset_clears_accumulated_distribution() {
    let mut meter = LraMeter::new();
    for _ in 0..20 {
        meter.push_short_term_mean_square(lufs_to_ms(-20.0));
    }
    for _ in 0..20 {
        meter.push_short_term_mean_square(lufs_to_ms(-30.0));
    }
    assert!(meter.lra_lu() > 1.0);
    meter.reset();
    assert_eq!(meter.lra_lu(), 0.0);
    for _ in 0..10 {
        meter.push_short_term_mean_square(lufs_to_ms(-23.0));
    }
    assert!(meter.lra_lu().abs() < 0.1);
}
