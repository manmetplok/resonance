//! End-to-end A1 reference parity (ba todo #1116): real A1 .nam files
//! must produce audio matching the NeuralAmpModelerCore reference compiled
//! WITH `enable_fast_tanh()` — the condition the official NAM plugin runs
//! under, i.e. what "correct" means for A1 files. Fixtures + provenance:
//! `tests/fixtures/a1/` (renders share the A2 input and render
//! conditions; harness in `tests/common/mod.rs`).
//!
//! These pins are the acceptance evidence for the user-approved A1 sound
//! change: before #1116 the legacy engine path measured max abs err 0.32
//! against this same reference on the same ±0.3 signal (doc #258).

mod common;

/// Measured 2026-07-28: wavenet_a1_standard 7.302e-7 max abs/rel err,
/// wavenet 1.509e-7 — build-noise level (the reference is gcc -O3 with
/// FMA contraction; the engine is rustc f32 without). 2e-6 covers that
/// noise with headroom while still failing loudly on any wiring or
/// activation-formula regression (those sit orders of magnitude higher:
/// the pre-#1116 structural error was 0.32, and the engine's previous
/// non-reference fast-tanh formula alone measured 4.4e-3).
const TOL: f32 = 2e-6;

fn assert_a1_parity(model: &str, reference: &str) {
    let input = common::read_f32(&common::fixture_path("input.f32"));
    let expected = common::read_f32(&common::fixture_path_in("a1", reference));
    let ours = common::run_nam_model(&common::fixture_path_in("a1", model), &input);
    let report = common::compare(&ours, &expected);
    println!(
        "{model} vs {reference}: max_abs {:.3e} @ {}, max_err {:.3e} @ {}",
        report.max_abs, report.max_abs_index, report.max_err, report.max_err_index
    );
    assert!(
        report.max_err <= TOL,
        "{model} diverges from {reference}: max_abs {:.3e} @ sample {}, max abs/rel err {:.3e} @ sample {} (tolerance {TOL:.1e})",
        report.max_abs,
        report.max_abs_index,
        report.max_err,
        report.max_err_index
    );
}

/// The standard A1 capture architecture (16/8 channels, dilations 1..512,
/// receptive field 4092). Upstream `my_model.nam` is byte-identical to
/// this file, so this test covers it too.
#[test]
fn wavenet_a1_standard_matches_fast_tanh_reference() {
    assert_a1_parity("wavenet_a1_standard.nam", "wavenet_a1_standard.fasttanh.f32");
}

#[test]
fn wavenet_small_matches_fast_tanh_reference() {
    assert_a1_parity("wavenet.nam", "wavenet.fasttanh.f32");
}
