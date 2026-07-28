//! End-to-end A2 reference parity (ba todo #1113): every committed A2
//! fixture must load through the public loader and produce audio matching
//! the NeuralAmpModelerCore reference render within tight tolerance.
//! Todo #1115 formalizes/extends this suite; the harness lives in
//! `tests/common/mod.rs`.
//!
//! The engine loads slimmable files at the full size, so the slimmable
//! fixtures compare against their `slim1` reference renders (byte-identical
//! to the reference's no-`--slim` default per the fixtures README).

mod common;

/// The reference computed in f64 with a single final f32 cast; the engine
/// is f32 throughout. 1e-6 mixed abs/rel covers the accumulation gap
/// (measured: slimmable_wavenet bit-identical, A2/condition_dsp within
/// 1 f32 ulp, wavenet_a2_max max rel 8.7e-7 — the same deviation a
/// locally rebuilt reference renderer shows against the committed
/// fixtures, i.e. compiler-flag noise) without hiding wiring errors,
/// which sit orders of magnitude higher.
const TOL: f32 = 1e-6;

#[test]
fn a2_container_full_matches_reference() {
    common::assert_reference_parity("A2.nam", "A2.slim1.f32", TOL);
}

#[test]
fn slimmable_wavenet_full_matches_reference() {
    common::assert_reference_parity("slimmable_wavenet.nam", "slimmable_wavenet.slim1.f32", TOL);
}

#[test]
fn wavenet_a2_max_matches_reference() {
    common::assert_reference_parity("wavenet_a2_max.nam", "wavenet_a2_max.default.f32", TOL);
}

#[test]
fn wavenet_condition_dsp_matches_reference() {
    common::assert_reference_parity(
        "wavenet_condition_dsp.nam",
        "wavenet_condition_dsp.default.f32",
        TOL,
    );
}

/// Ad-hoc comparison of an arbitrary model against an arbitrary reference
/// render, for debugging parity work outside the committed fixture set
/// (e.g. A1 models rendered with the reference `tools/render`):
/// `NAM_DIAG_MODEL=/path/model.nam NAM_DIAG_REF=/path/ref.f32 \
///  NAM_DIAG_INPUT=/path/input.f32 cargo test -p resonance-amp \
///  --test nam_a2_reference_parity -- --ignored --nocapture`
#[test]
#[ignore = "diagnostic; needs NAM_DIAG_* env vars"]
fn diag_compare_env() {
    let model = std::env::var("NAM_DIAG_MODEL").expect("NAM_DIAG_MODEL not set");
    let reference = std::env::var("NAM_DIAG_REF").expect("NAM_DIAG_REF not set");
    let input_path = std::env::var("NAM_DIAG_INPUT")
        .unwrap_or_else(|_| common::fixture_path("input.f32"));
    let input = common::read_f32(&input_path);
    let expected = common::read_f32(&reference);
    let ours = common::run_nam_model(&model, &input);
    let report = common::compare(&ours, &expected);
    println!(
        "{model} vs {reference}: max_abs {:.3e} @ {}, max_err {:.3e} @ {}",
        report.max_abs, report.max_abs_index, report.max_err, report.max_err_index
    );
    for i in [0usize, 1, 2, 1024, 1025, 2048, 4095] {
        println!(
            "  [{i}] ours {:+.7} ref {:+.7} diff {:+.3e}",
            ours[i],
            expected[i],
            ours[i] - expected[i]
        );
    }
}
