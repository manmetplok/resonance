//! End-to-end A2 reference parity (ba todos #1113 + #1115): every
//! committed A2 fixture must load through the public loader and produce
//! audio matching the NeuralAmpModelerCore reference render within tight
//! tolerance. Parity conditions (f64 reference, prewarm semantics,
//! 64-block equivalence, tolerance rationale): `tests/common/mod.rs`
//! module docs. Companion suites: `nam_a1_reference_parity` (A1 WaveNet,
//! fast-tanh reference) and `nam_lstm_reference_parity` (LSTM).
//!
//! The engine loads slimmable files at the full (A2-Full) size only —
//! runtime Lite-slice selection is not implemented (todo #1112 scoped v1
//! to the full slice; requesting a non-full size is rejected at the
//! `extract_slimmed_weights` layer, covered in `nam_slimmable.rs`). The
//! slimmable fixtures therefore compare against their `slim1` reference
//! renders (byte-identical to the reference's no-`--slim` default per the
//! fixtures README); the committed `slim0` (Lite) reference vectors are
//! the ready-made acceptance data for future Lite support and are only
//! integrity-checked here.

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

/// Every A2 fixture — FiLM, gating/blending, grouped convs, head1x1,
/// windowed heads, condition_dsp between them — renders bit-identically
/// through `process_block` at every split (see
/// `common::assert_block_split_invariance`).
#[test]
fn block_processing_matches_sample_serial_bit_for_bit() {
    let input = common::read_f32(&common::fixture_path("input.f32"));
    for model in [
        "A2.nam",
        "slimmable_wavenet.nam",
        "wavenet_a2_max.nam",
        "wavenet_condition_dsp.nam",
    ] {
        common::assert_block_split_invariance(&common::fixture_path(model), &input);
    }
}

/// The Lite-slice reference vectors must stay distinct from the Full
/// renders they sit next to: if a regeneration ever overwrote a `slim0`
/// vector with a full-size render (or vice versa), the future Lite
/// acceptance data would silently become meaningless. Also pins the
/// shared 4096-sample window for every committed vector.
#[test]
fn lite_reference_vectors_are_intact_for_future_lite_support() {
    for (slim0, slim1) in [
        ("A2.slim0.f32", "A2.slim1.f32"),
        ("slimmable_wavenet.slim0.f32", "slimmable_wavenet.slim1.f32"),
    ] {
        let lite = common::read_f32(&common::fixture_path(slim0));
        let full = common::read_f32(&common::fixture_path(slim1));
        assert_eq!(lite.len(), 4096, "{slim0}: reference window must be 4096");
        assert_eq!(full.len(), 4096, "{slim1}: reference window must be 4096");
        assert!(
            lite.iter().zip(&full).any(|(a, b)| a.to_bits() != b.to_bits()),
            "{slim0} is byte-identical to {slim1}: the Lite reference render was overwritten"
        );
    }
}

/// Ad-hoc comparison of an arbitrary model against an arbitrary reference
/// render, for debugging parity work outside the committed fixture set —
/// works for every fixture family (A2, A1, LSTM) since it takes explicit
/// paths; pass your own `NAM_DIAG_INPUT` if the model was not rendered
/// over the shared `a2/input.f32`:
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
