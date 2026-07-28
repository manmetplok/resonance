//! End-to-end LSTM reference parity (ba todo #1115): the real NAM LSTM
//! example model must produce audio matching the NeuralAmpModelerCore
//! reference compiled WITH `enable_fast_tanh()` — the condition the
//! official NAM plugin runs under, same rationale as the A1 WaveNet
//! suite. Fixtures + provenance: `tests/fixtures/lstm/`; parity
//! conditions: `tests/common/mod.rs` module docs.
//!
//! Unlike the WaveNet suites this one prewarms with EXACTLY the
//! reference's zero count (24000 = 0.5 s at 48 kHz): an LSTM is
//! recurrent, so its state depends (asymptotically) on how many zeros it
//! was fed.
//!
//! Before todo #1115 there was NO LSTM output coverage at all — and the
//! engine's LSTM weight layout was structurally wrong (PyTorch-style
//! separate w_ih/w_hh + two biases + zero initial state + trailing
//! head_scale, vs the actual NAM export: combined `[4h, in+h]` matrix,
//! one summed bias, LEARNED initial h0/c0, no head scale) — real NAM
//! LSTM files could not even load (weight-count mismatch: the engine
//! wanted 72 recurrent weights from `lstm.nam`'s 70-weight blob).

mod common;

/// Measured 2026-07-28: 2.757e-7 max abs/mixed err (hidden size 3, one
/// layer — tiny model, error stays at f32 build-noise level). 1e-6
/// matches the A2 suite bound and fails loudly on any layout,
/// activation-formula, initial-state, or prewarm regression (loading the
/// weights in the wrong order measures O(1e-1); the pre-#1116 Padé
/// fast-tanh alone measured O(1e-3) on WaveNet signals).
const TOL: f32 = 1e-6;

#[test]
fn lstm_matches_fast_tanh_reference() {
    let input = common::read_f32(&common::fixture_path("input.f32"));
    let expected = common::read_f32(&common::fixture_path_in("lstm", "lstm.fasttanh.f32"));
    let ours = common::run_nam_model_with_prewarm(
        &common::fixture_path_in("lstm", "lstm.nam"),
        &input,
        common::LSTM_PREWARM_SAMPLES_48K,
    );
    let report = common::compare(&ours, &expected);
    println!(
        "lstm.nam vs lstm.fasttanh.f32: max_abs {:.3e} @ {}, max_err {:.3e} @ {}",
        report.max_abs, report.max_abs_index, report.max_err, report.max_err_index
    );
    assert!(
        report.max_err <= TOL,
        "lstm.nam diverges from lstm.fasttanh.f32: max_abs {:.3e} @ sample {}, max abs/rel err {:.3e} @ sample {} (tolerance {TOL:.1e})",
        report.max_abs,
        report.max_abs_index,
        report.max_err,
        report.max_err_index
    );
}

/// The reference prewarm count is not incidental: the learned initial
/// state decays toward the zero-input fixed point during prewarm, and for
/// a recurrent model the parity conditions pin the exact count. This
/// guards the harness itself — if the fixed point were not yet reached at
/// 24000 samples, a "generous constant" prewarm (as used for the FIR
/// WaveNet suites) would silently compare a different state.
#[test]
fn lstm_prewarm_reaches_fixed_point_at_reference_count() {
    let input = common::read_f32(&common::fixture_path("input.f32"));
    let model_path = common::fixture_path_in("lstm", "lstm.nam");
    let at_reference =
        common::run_nam_model_with_prewarm(&model_path, &input, common::LSTM_PREWARM_SAMPLES_48K);
    let generous =
        common::run_nam_model_with_prewarm(&model_path, &input, common::PREWARM_SAMPLES);
    let report = common::compare(&generous, &at_reference);
    assert!(
        report.max_abs == 0.0,
        "LSTM state not settled at the reference prewarm count: extra zeros change the render by {:.3e} @ sample {}",
        report.max_abs,
        report.max_abs_index
    );
}
