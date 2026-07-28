//! Shared NAM reference-parity harness (ba todo #1113, formalized by todo
//! #1115). This header is THE definition of the parity conditions; the
//! `nam_a2_reference_parity`, `nam_a1_reference_parity` and
//! `nam_lstm_reference_parity` suites (and the fixture READMEs under
//! `tests/fixtures/{a1,a2,lstm}/`) all reference it.
//!
//! Reproduces the exact conditions the reference output vectors were
//! rendered under (NeuralAmpModelerCore `tools/render` at commit 3cde95c,
//! see the fixture READMEs for provenance and exact commands):
//!
//! - `DSP::Reset(48000.0, 64)` with prewarm-on-reset: the model is fed
//!   zeros before the first real sample, in 64-sample blocks, until at
//!   least `GetPrewarmSamples()` zeros have been processed.
//!   - WaveNet: `1 + sum of receptive fields` zeros, rounded up to whole
//!     blocks. A WaveNet is a finite-impulse-response network, so its
//!     state reaches a fixed point once the zero run covers the receptive
//!     field — feeding MORE zeros leaves the state bit-identical. The
//!     harness therefore uses one generous constant
//!     ([`PREWARM_SAMPLES`]) instead of re-deriving each model's
//!     receptive field.
//!   - LSTM: exactly `0.5 * sample_rate` zeros (24000 at 48 kHz; 375
//!     whole 64-blocks, so no rounding). An LSTM is recurrent — its
//!     state only converges asymptotically under zero input — so the
//!     LSTM suite feeds EXACTLY the reference count
//!     ([`LSTM_PREWARM_SAMPLES_48K`]) rather than the generous constant.
//! - 64-sample blocks: the engine is sample-serial, so block size cannot
//!   change its output; the reference's blocked processing is likewise
//!   stream-equivalent. This is why the harness has no block-size
//!   dimension to vary — sample-serial processing IS every block size.
//! - The reference computes in f64 (`NAM_SAMPLE = double`) with one final
//!   f32 cast per sample; this engine is f32 throughout, so comparisons
//!   use a small tolerance, never bit equality. Per-suite tolerances and
//!   the measured errors that justify them are documented on each
//!   suite's `TOL` constant; all sit at f32 build-noise level
//!   (accumulation-order / FMA-contraction differences), orders of
//!   magnitude below any wiring error.
//! - Activation flavor: A2-marked models use the exact activations and
//!   compare against default reference renders; A1 WaveNet and LSTM
//!   models use the fast flavor (the official NAM plugin runs
//!   `enable_fast_tanh()`) and compare against `--fast-tanh` renders
//!   (see `tests/fixtures/a1/README.md` for the render-tool patch).

// Compiled once per test binary; each suite uses a subset of the helpers.
#![allow(dead_code)]

use resonance_amp::nam::parse::{load_model_from_file, LoadedModel};

/// Zero-input samples fed after reset before the comparison window.
/// Must exceed every fixture's receptive field (the largest, the A1
/// standard config, has RF 4092; the A2 fixtures are all far smaller).
pub const PREWARM_SAMPLES: usize = 1 << 15;

/// Exact prewarm the reference feeds a 48 kHz LSTM: `GetPrewarmSamples()
/// = 0.5 * 48000`, already a whole number of 64-blocks.
pub const LSTM_PREWARM_SAMPLES_48K: usize = 24_000;

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
    run_nam_model_with_prewarm(model_path, input, PREWARM_SAMPLES)
}

/// [`run_nam_model`] with an explicit prewarm length, for recurrent
/// models where the exact reference count matters (see the module docs).
pub fn run_nam_model_with_prewarm(
    model_path: &str,
    input: &[f32],
    prewarm_samples: usize,
) -> Vec<f32> {
    let LoadedModel { mut model, .. } = load_model_from_file(model_path)
        .unwrap_or_else(|e| panic!("failed to load {model_path}: {e}"));
    model.reset();
    for _ in 0..prewarm_samples {
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
