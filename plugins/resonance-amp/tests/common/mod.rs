//! Shared NAM reference-parity harness (ba todo #1113; the A2 fixture
//! parity suite of todo #1115 builds on this).
//!
//! Reproduces the exact conditions the reference outputs in
//! `tests/fixtures/a2/` were rendered under (NeuralAmpModelerCore
//! `tools/render` at commit 3cde95c, see the fixtures README):
//!
//! - `DSP::Reset(48000.0, 64)` with prewarm-on-reset: the model is fed
//!   zeros over (at least) its receptive field before the first real
//!   sample. The reference feeds `ceil((1 + sum of receptive fields) /
//!   64) * 64` zeros; a WaveNet is a finite-impulse-response network, so
//!   its state reaches a fixed point once the zero run covers the
//!   receptive field — feeding MORE zeros leaves the state bit-identical.
//!   The harness therefore uses one generous constant instead of
//!   re-deriving each model's receptive field.
//! - 64-sample blocks: the engine is sample-serial, so block size cannot
//!   change its output; the reference's blocked processing is likewise
//!   stream-equivalent.
//! - The reference computes in f64 (`NAM_SAMPLE = double`) with one final
//!   f32 cast per sample; this engine is f32 throughout, so comparisons
//!   use a small tolerance, never bit equality.

use resonance_amp::nam::parse::{load_model_from_file, LoadedModel};

/// Zero-input samples fed after reset before the comparison window.
/// Must exceed every fixture's receptive field (the largest, the A1
/// standard config, has RF 4092; the A2 fixtures are all far smaller).
pub const PREWARM_SAMPLES: usize = 1 << 15;

/// Absolute path of a file in `tests/fixtures/<dir>/`.
pub fn fixture_path_in(dir: &str, name: &str) -> String {
    format!(
        "{}/tests/fixtures/{dir}/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Absolute path of a file in `tests/fixtures/a2/`.
pub fn fixture_path(name: &str) -> String {
    fixture_path_in("a2", name)
}

/// Read a raw little-endian f32 vector (the fixture input/output format).
pub fn read_f32(path: &str) -> Vec<f32> {
    let bytes =
        std::fs::read(path).unwrap_or_else(|e| panic!("failed to read f32 vector {path}: {e}"));
    assert_eq!(
        bytes.len() % 4,
        0,
        "f32 vector {path} has a truncated sample"
    );
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Load a .nam model and run it under the reference conditions: reset,
/// prewarm on zeros, then process `input` sample-serially.
pub fn run_nam_model(model_path: &str, input: &[f32]) -> Vec<f32> {
    let LoadedModel { mut model, .. } = load_model_from_file(model_path)
        .unwrap_or_else(|e| panic!("failed to load {model_path}: {e}"));
    model.reset();
    for _ in 0..PREWARM_SAMPLES {
        model.process_sample(0.0);
    }
    input.iter().map(|&x| model.process_sample(x)).collect()
}

/// Worst-case deviation between an engine render and a reference vector.
pub struct ParityReport {
    pub max_abs: f32,
    pub max_abs_index: usize,
    /// Maximum of `|ours - reference| / max(1, |reference|)`: absolute
    /// error for small samples, relative error for large ones.
    pub max_err: f32,
    pub max_err_index: usize,
}

pub fn compare(ours: &[f32], reference: &[f32]) -> ParityReport {
    assert_eq!(
        ours.len(),
        reference.len(),
        "engine and reference vectors must have equal length"
    );
    let mut report = ParityReport {
        max_abs: 0.0,
        max_abs_index: 0,
        max_err: 0.0,
        max_err_index: 0,
    };
    for (i, (&a, &b)) in ours.iter().zip(reference).enumerate() {
        assert!(a.is_finite(), "engine output not finite at sample {i}: {a}");
        let abs = (a - b).abs();
        if abs > report.max_abs {
            report.max_abs = abs;
            report.max_abs_index = i;
        }
        let err = abs / b.abs().max(1.0);
        if err > report.max_err {
            report.max_err = err;
            report.max_err_index = i;
        }
    }
    report
}

/// Render `model` over the shared fixture input and assert its output
/// matches `reference` (a fixture .f32 name) within `tol` (mixed
/// absolute/relative, see [`ParityReport::max_err`]).
pub fn assert_reference_parity(model: &str, reference: &str, tol: f32) {
    let input = read_f32(&fixture_path("input.f32"));
    let expected = read_f32(&fixture_path(reference));
    let ours = run_nam_model(&fixture_path(model), &input);
    let report = compare(&ours, &expected);
    println!(
        "{model} vs {reference}: max_abs {:.3e} @ {}, max_err {:.3e} @ {}",
        report.max_abs, report.max_abs_index, report.max_err, report.max_err_index
    );
    assert!(
        report.max_err <= tol,
        "{model} diverges from {reference}: max_abs {:.3e} @ sample {}, max abs/rel err {:.3e} @ sample {} (tolerance {tol:.1e})",
        report.max_abs,
        report.max_abs_index,
        report.max_err,
        report.max_err_index
    );
}
