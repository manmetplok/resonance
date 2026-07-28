//! Reset-then-render determinism under the parity conditions (ba todo
//! #1115): rendering the same input twice on the SAME model instance,
//! with a reset + prewarm between, must be bit-identical to the first
//! render. This proves `reset()` returns every piece of model state to
//! its initial value — ring buffers and head chains for WaveNet, FiLM
//! and condition_dsp state, and the LEARNED initial h0/c0 for LSTM
//! (whose state, unlike a FIR WaveNet's, would carry an audible residue
//! of the previous render if reset were incomplete).
//!
//! One fixture per family, chosen for maximal state surface:
//! - A2: `wavenet_a2_max.nam` (bottleneck, FiLM at all 8 insertion
//!   points, condition_dsp, grouped convs, windowed head)
//! - A1: `wavenet_a1_standard.nam` (the real capture architecture,
//!   receptive field 4092)
//! - LSTM: `lstm.nam` (recurrent state + learned initial state)
//!
//! Parity conditions (prewarm semantics, per-family zero counts):
//! `tests/common/mod.rs` module docs.

mod common;

use resonance_amp::nam::parse::load_model_from_file;

fn assert_reset_determinism(model_path: &str, prewarm_samples: usize) {
    let input = common::read_f32(&common::fixture_path("input.f32"));
    let mut loaded = load_model_from_file(model_path)
        .unwrap_or_else(|e| panic!("failed to load {model_path}: {e}"));

    let mut render = || -> Vec<f32> {
        loaded.model.reset();
        for _ in 0..prewarm_samples {
            loaded.model.process_sample(0.0);
        }
        input.iter().map(|&x| loaded.model.process_sample(x)).collect()
    };

    let first = render();
    let second = render();
    for (i, (a, b)) in first.iter().zip(&second).enumerate() {
        assert!(
            a.to_bits() == b.to_bits(),
            "{model_path}: render after reset is not bit-identical at sample {i}: {a:?} vs {b:?}",
        );
    }
}

#[test]
fn a2_max_reset_render_is_bit_deterministic() {
    assert_reset_determinism(
        &common::fixture_path("wavenet_a2_max.nam"),
        common::PREWARM_SAMPLES,
    );
}

#[test]
fn a1_standard_reset_render_is_bit_deterministic() {
    assert_reset_determinism(
        &common::fixture_path_in("a1", "wavenet_a1_standard.nam"),
        common::PREWARM_SAMPLES,
    );
}

#[test]
fn lstm_reset_render_is_bit_deterministic() {
    assert_reset_determinism(
        &common::fixture_path_in("lstm", "lstm.nam"),
        common::LSTM_PREWARM_SAMPLES_48K,
    );
}
