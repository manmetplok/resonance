//! Tests for the A2 `condition_dsp` sub-network (ba todo #1110).
//!
//! The nested model is a complete .nam-style object under the outer config
//! (`architecture`/`config`/`weights`/`sample_rate`). Reference semantics
//! (NAM/wavenet/model.cpp): it is built eagerly and recursively from that
//! object — its weights come from the nested object's OWN `weights` array,
//! never from the outer flat stream (`set_weights_`: "condition_dsp already
//! has its own weights from construction") — it runs on the raw model input
//! once per sample before the layer arrays, and its multi-channel output
//! becomes the condition every array's input mixin and FiLM sites read.
//! Covers: hand-computed condition wiring, the outer weight-stream position
//! pin, real-fixture construction with exact weight consumption
//! (`wavenet_condition_dsp.nam`, `wavenet_a2_max.nam`), reset of nested
//! state, width/architecture/sample-rate validation errors, and
//! absent-condition_dsp passthrough.

use serde_json::{json, Value};

use resonance_amp::nam::parse::{load_model_from_file, parse_wavenet_config, WeightReader};
use resonance_amp::nam::wavenet::WaveNetModel;
use resonance_amp::nam::NamInference;

fn fixture_json(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/a2/{name}", env!("CARGO_MANIFEST_DIR"));
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"));
    serde_json::from_str(&data).unwrap_or_else(|e| panic!("fixture {path} is not JSON: {e}"))
}

fn fixture_path(name: &str) -> String {
    format!("{}/tests/fixtures/a2/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn fixture_weights(file: &Value) -> Vec<f32> {
    file["weights"]
        .as_array()
        .expect("weights array")
        .iter()
        .map(|w| w.as_f64().unwrap() as f32)
        .collect()
}

/// Minimal single-stack, single-layer, kernel-1 ungated ReLU config (new
/// format, so layer1x1 is active). Weight consumption order: conv (1),
/// conv bias (1), input_mixin (1), layer1x1 w+b (2), head rechannel (1),
/// head_scale (1) = 7 weights. With positive inputs and weights every ReLU
/// is the identity, so outputs are exact in f32.
fn tiny_config(input_size: usize, condition_size: usize, head_size: usize) -> Value {
    json!({
        "layers": [{
            "input_size": input_size,
            "condition_size": condition_size,
            "head_size": head_size,
            "channels": 1,
            "kernel_size": 1,
            "dilations": [1],
            "activation": "ReLU",
            "gated": false,
            "head_bias": false
        }],
        "head": null,
        "head_scale": 0.02
    })
}

/// A nested condition_dsp model object wrapping `config` + `weights`.
fn nested_model(config: Value, weights: &[f32]) -> Value {
    json!({
        "version": "0.6.0",
        "architecture": "WaveNet",
        "config": config,
        "weights": weights,
        "sample_rate": 48000
    })
}

fn with_condition_dsp(mut config: Value, nested: Value) -> Value {
    config["condition_dsp"] = nested;
    config
}

fn build(config: Value, weights: &[f32]) -> Result<(WaveNetModel, usize), String> {
    let cfg = parse_wavenet_config(config)?;
    let mut reader = WeightReader::new(weights);
    let model = WaveNetModel::from_config_and_weights(cfg, &mut reader)?;
    Ok((model, reader.remaining()))
}

/// Construction expected to fail; returns the error message.
fn build_err(config: Value, weights: &[f32], what: &str) -> String {
    match build(config, weights) {
        Ok(_) => panic!("{what}: expected construction to fail"),
        Err(e) => e,
    }
}

/// Nested net N with weights [conv=1, bias=0, mixin=1, l1x1=0, l1x1_b=0,
/// head=1, head_scale=2]: for x > 0, z = relu(x + x) = 2x, head path
/// relu(2x) = 2x, output N(x) = 2 * 2x = 4x.
const NESTED_WEIGHTS: [f32; 7] = [1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 2.0];

/// Outer net O with weights [conv=1, bias=0, mixin=1, l1x1=0, l1x1_b=0,
/// head=1, head_scale=1]: with condition c(x), z = relu(x + c(x)), output
/// O(x) = relu(z). With condition_dsp: c(x) = 4x so O(x) = 5x. Without:
/// c(x) = x (post-rechannel snapshot) so O(x) = 2x.
const OUTER_WEIGHTS: [f32; 7] = [1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0];

// -- Hand-computed wiring -----------------------------------------------------

/// The nested output must replace the condition the input_mixin reads:
/// O(0.5) = 5 * 0.5 = 2.5 exactly. If the condition were still the raw
/// input the result would be 1.0; if the nested net had (wrongly) consumed
/// the outer stream's weights (head_scale 1 instead of 2) it would be 1.5.
#[test]
fn condition_dsp_output_feeds_input_mixin() {
    let nested = nested_model(tiny_config(1, 1, 1), &NESTED_WEIGHTS);
    let config = with_condition_dsp(tiny_config(1, 1, 1), nested);
    let (mut model, remaining) = build(config, &OUTER_WEIGHTS).expect("construction");
    assert_eq!(remaining, 0, "outer stream fully consumed");
    assert_eq!(model.process_sample(0.5), 2.5);
    assert_eq!(model.process_sample(0.25), 1.25);
}

/// Absent condition_dsp: the condition stays the engine's post-rechannel
/// snapshot — O(x) = 2x, today's A1 behavior — and an explicit JSON `null`
/// parses identically to the key being absent.
#[test]
fn absent_condition_dsp_is_passthrough() {
    let (mut plain, remaining) = build(tiny_config(1, 1, 1), &OUTER_WEIGHTS).expect("construction");
    assert_eq!(remaining, 0);
    assert_eq!(plain.process_sample(0.5), 1.0);

    let cfg_null = parse_wavenet_config(with_condition_dsp(tiny_config(1, 1, 1), Value::Null))
        .expect("null condition_dsp parses");
    assert!(cfg_null.condition_dsp.is_none(), "null == absent");
    let mut reader = WeightReader::new(&OUTER_WEIGHTS);
    let mut from_null =
        WaveNetModel::from_config_and_weights(cfg_null, &mut reader).expect("construction");
    for i in 1..16 {
        let x = i as f32 / 16.0;
        assert_eq!(plain.process_sample(x), from_null.process_sample(x));
    }
}

/// Weight-stream position pin: the nested weights live in the nested model
/// object, NOT in the outer flat stream (before OR after the main arrays).
/// The outer stream of a model with a condition_dsp is therefore exactly as
/// long as without one; any extra outer weights stay unconsumed.
#[test]
fn nested_weights_are_not_taken_from_the_outer_stream() {
    let nested = nested_model(tiny_config(1, 1, 1), &NESTED_WEIGHTS);

    // Exactly the no-condition_dsp weight count constructs and consumes all.
    let (_, remaining) =
        build(with_condition_dsp(tiny_config(1, 1, 1), nested.clone()), &OUTER_WEIGHTS)
            .expect("construction");
    assert_eq!(remaining, 0);

    // Appending the nested net's 7 weights to the outer stream must leave
    // them unconsumed (a wrong "nested weights after the main arrays"
    // reading would eat them).
    let mut padded = OUTER_WEIGHTS.to_vec();
    padded.extend_from_slice(&NESTED_WEIGHTS);
    let (_, remaining) = build(with_condition_dsp(tiny_config(1, 1, 1), nested), &padded)
        .expect("construction");
    assert_eq!(remaining, NESTED_WEIGHTS.len(), "extra outer weights untouched");

    // A wrong "nested weights before the main arrays" reading would
    // underflow the exact-length stream — covered by the first assert
    // (construction succeeded with only the outer 7).
}

/// The nested weight array itself must be consumed exactly.
#[test]
fn nested_weight_surplus_is_rejected() {
    let mut too_many = NESTED_WEIGHTS.to_vec();
    too_many.push(0.0);
    let nested = nested_model(tiny_config(1, 1, 1), &too_many);
    let err = build_err(
        with_condition_dsp(tiny_config(1, 1, 1), nested),
        &OUTER_WEIGHTS,
        "surplus nested weights",
    );
    assert!(
        err.contains("condition_dsp") && err.contains("unused weights"),
        "unexpected error: {err}"
    );

    let too_few = &NESTED_WEIGHTS[..5];
    let nested = nested_model(tiny_config(1, 1, 1), too_few);
    let err = build_err(
        with_condition_dsp(tiny_config(1, 1, 1), nested),
        &OUTER_WEIGHTS,
        "missing nested weights",
    );
    assert!(err.contains("condition_dsp"), "unexpected error: {err}");
}

// -- Multi-channel nested output ----------------------------------------------

/// `process_sample_into` exposes the full multi-channel, head-scaled output
/// (reference `wave_net_output_channels`: last array's head width without a
/// head MLP). Single-stack net with head_size 2, head rechannel weights
/// [1.0, 0.5]: out = [2x, x].
#[test]
fn multi_channel_output_matches_hand_computation() {
    // conv=1, bias=0, mixin=1, l1x1 w=0 b=0, head rechannel [1.0, 0.5],
    // head_scale=1.
    let weights = [1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.5, 1.0];
    let (mut model, remaining) = build(tiny_config(1, 1, 2), &weights).expect("construction");
    assert_eq!(remaining, 0);
    assert_eq!(model.out_channels(), 2);
    let mut out = [0.0f32; 2];
    model.process_sample_into(0.5, &mut out);
    assert_eq!(out, [1.0, 0.5]);
}

// -- Validation ---------------------------------------------------------------

/// Nested output width must equal every stack's condition_size (reference
/// WaveNet ctor assert).
#[test]
fn condition_width_mismatch_is_rejected() {
    let nested = nested_model(tiny_config(1, 1, 1), &NESTED_WEIGHTS);
    let err = build_err(
        with_condition_dsp(tiny_config(1, 2, 1), nested),
        &OUTER_WEIGHTS,
        "condition width mismatch",
    );
    assert!(
        err.contains("condition_size") && err.contains("condition DSP"),
        "unexpected error: {err}"
    );
}

/// Nested input width must equal the WaveNet's own input channels
/// (reference: the condition DSP consumes the raw model input).
#[test]
fn nested_input_width_mismatch_is_rejected() {
    // input_size 2 -> a 2->1 rechannel (2 weights) precedes the usual 7.
    let nested_weights = [1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 2.0];
    let nested = nested_model(tiny_config(2, 1, 1), &nested_weights);
    let err = build_err(
        with_condition_dsp(tiny_config(1, 1, 1), nested),
        &OUTER_WEIGHTS,
        "input width mismatch",
    );
    assert!(
        err.contains("input channels") && err.contains("condition DSP"),
        "unexpected error: {err}"
    );
}

/// A2 training only emits WaveNet sub-networks; other nested architectures
/// get a clear unsupported error.
#[test]
fn non_wavenet_condition_dsp_is_rejected() {
    let nested = json!({
        "version": "0.6.0",
        "architecture": "LSTM",
        "config": { "input_size": 1, "hidden_size": 2, "num_layers": 1 },
        "weights": [],
        "sample_rate": 48000
    });
    let err = build_err(
        with_condition_dsp(tiny_config(1, 1, 1), nested),
        &OUTER_WEIGHTS,
        "non-WaveNet nested model",
    );
    assert!(
        err.contains("Unsupported condition_dsp architecture") && err.contains("LSTM"),
        "unexpected error: {err}"
    );
}

/// A condition_dsp trained at a different rate than the outer model is a
/// broken export (reference `parse_config_json` sample-rate assert).
#[test]
fn nested_sample_rate_mismatch_is_rejected() {
    let mut nested = nested_model(tiny_config(1, 1, 1), &NESTED_WEIGHTS);
    nested["sample_rate"] = json!(44100);
    let file = json!({
        "version": "0.6.0",
        "architecture": "WaveNet",
        "config": with_condition_dsp(tiny_config(1, 1, 1), nested),
        "weights": OUTER_WEIGHTS.to_vec(),
        "sample_rate": 48000
    });
    let path = std::env::temp_dir().join(format!(
        "resonance_amp_nam_condition_dsp_rate_{}.nam",
        std::process::id()
    ));
    std::fs::write(&path, serde_json::to_string(&file).unwrap()).unwrap();
    let err = load_model_from_file(path.to_str().unwrap()).map(|_| ()).expect_err("rate mismatch");
    let _ = std::fs::remove_file(&path);
    assert!(err.contains("sample rate"), "unexpected error: {err}");
}

// -- Real fixtures ------------------------------------------------------------

/// `wavenet_condition_dsp.nam`: outer A1-style net conditioned on a nested
/// 2-array WaveNet (1 -> 3 channels). Construction must consume the outer
/// stream exactly (147 weights, identical layout to a condition_dsp-less
/// file) and the nested stream exactly (137 weights, from the nested
/// object), and inference must produce finite output.
#[test]
fn condition_dsp_fixture_constructs_and_consumes_exactly() {
    let file = fixture_json("wavenet_condition_dsp.nam");
    let weights = fixture_weights(&file);
    let (mut model, remaining) =
        build(file["config"].clone(), &weights).expect("fixture construction");
    assert_eq!(remaining, 0, "outer stream fully consumed");
    assert_eq!(model.out_channels(), 1);
    for i in 0..256 {
        let x = (i as f32 / 64.0).sin() * 0.5;
        assert!(model.process_sample(x).is_finite());
    }

    // Full-file path (includes the nested sample-rate check).
    let loaded = load_model_from_file(&fixture_path("wavenet_condition_dsp.nam"))
        .expect("fixture loads through the file loader");
    assert_eq!(loaded.sample_rate, 48_000.0);
}

/// `wavenet_a2_max.nam`: the maximal A2 fixture — its condition_dsp is a
/// full A2 WaveNet itself (bottleneck, per-layer activation arrays, blended
/// gating, head1x1, FiLM; 1 -> 8 channels, 1052 nested weights).
#[test]
fn a2_max_fixture_constructs_and_consumes_exactly() {
    let file = fixture_json("wavenet_a2_max.nam");
    let weights = fixture_weights(&file);
    let (mut model, remaining) =
        build(file["config"].clone(), &weights).expect("fixture construction");
    assert_eq!(remaining, 0, "outer stream fully consumed");
    for i in 0..256 {
        let x = (i as f32 / 48.0).sin() * 0.25;
        assert!(model.process_sample(x).is_finite());
    }
}

// -- Reset --------------------------------------------------------------------

/// The nested net is stateful (its conv rings hold history); reset must
/// clear that state too, restoring the exact post-construction outputs.
#[test]
fn reset_clears_nested_state() {
    let file = fixture_json("wavenet_condition_dsp.nam");
    let weights = fixture_weights(&file);
    let (mut model, _) = build(file["config"].clone(), &weights).expect("fixture construction");

    let input: Vec<f32> = (0..64).map(|i| ((i as f32) * 0.37).sin() * 0.5).collect();
    let first: Vec<f32> = input.iter().map(|&x| model.process_sample(x)).collect();

    // Without a reset the rings (outer + nested) carry history: the same
    // input must NOT reproduce the first run.
    let carried: Vec<f32> = input.iter().map(|&x| model.process_sample(x)).collect();
    assert_ne!(first, carried, "model must be stateful across runs");

    // After reset the outputs are bit-identical to the fresh model's.
    model.reset();
    let after_reset: Vec<f32> = input.iter().map(|&x| model.process_sample(x)).collect();
    assert_eq!(first, after_reset, "reset must clear nested state too");
}
