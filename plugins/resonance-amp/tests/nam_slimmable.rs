//! Slimmable packed-weight unpacking (ba todo #1112).
//!
//! Covers the A2 slimmable surface at load time, v1 = full-size (A2-Full)
//! selection: the `slice_channels_uniform` extraction walk (pinned against
//! hand-computed packed layouts for BOTH sizes of tiny two-size configs),
//! sliced-config derivation, full-slice loading of the real slimmable
//! fixtures (`slimmable_wavenet.nam` packed WaveNet, `A2.nam`
//! SlimmableContainer) with exact weight consumption, non-slimmable
//! passthrough bit-identity, and the validation errors (unknown method,
//! descriptor inconsistencies, container shape).
//!
//! Fixtures are the official NeuralAmpModelerCore example models (MIT); see
//! tests/fixtures/a2/README.md.

use serde_json::{json, Value};

use resonance_amp::nam::parse::{load_model_from_file, parse_full_wavenet_config};
use resonance_amp::nam::wavenet::slimmable::{
    channels_for_size, derive_params_for_channels, extract_slimmed_weights, is_full_size,
    FULL_SIZE,
};
use resonance_amp::nam::NamInference;

// -- Helpers ------------------------------------------------------------------

fn fixture_path(name: &str) -> String {
    format!("{}/tests/fixtures/a2/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn fixture_json(name: &str) -> Value {
    let data = std::fs::read_to_string(fixture_path(name)).expect("fixture readable");
    serde_json::from_str(&data).expect("fixture is JSON")
}

fn write_temp_nam(name: &str, file: &Value) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "resonance_amp_nam_slimmable_{}_{name}.nam",
        std::process::id()
    ));
    std::fs::write(&path, serde_json::to_string(file).unwrap()).unwrap();
    path
}

fn load_value(name: &str, file: &Value) -> Result<Box<dyn NamInference>, String> {
    let path = write_temp_nam(name, file);
    let result = load_model_from_file(path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);
    result.map(|loaded| loaded.model)
}

fn load_err(name: &str, file: &Value) -> String {
    load_value(name, file)
        .map(|_| ())
        .expect_err("expected the load to fail")
}

/// Bit-exact output comparison over a deterministic input ramp.
fn assert_outputs_bit_identical(
    a: &mut dyn NamInference,
    b: &mut dyn NamInference,
    samples: usize,
    what: &str,
) {
    for n in 0..samples {
        let x = ((n as f32) * 0.001).sin() * 0.5;
        let ya = a.process_sample(x);
        let yb = b.process_sample(x);
        assert_eq!(
            ya.to_bits(),
            yb.to_bits(),
            "{what}: sample {n} diverged ({ya} vs {yb})"
        );
    }
}

fn seq_weights(n: usize) -> Vec<f32> {
    (0..n).map(|i| i as f32).collect()
}

/// Tiny two-size, two-array slimmable config exercising the cross-array
/// couplings (rechannel input columns follow the previous array's target,
/// intermediate head rows follow the next array's target).
fn two_array_slim_config() -> Value {
    json!({
        "layers": [
            {"input_size": 1, "condition_size": 1, "head_size": 2, "channels": 3,
             "dilations": [1], "kernel_size": 1, "activation": "ReLU", "gated": false,
             "head_bias": true,
             "slimmable": {"method": "slice_channels_uniform",
                           "kwargs": {"allowed_channels": [1, 3]}}},
            {"input_size": 3, "condition_size": 1, "head_size": 1, "channels": 2,
             "dilations": [1], "kernel_size": 1, "activation": "ReLU", "gated": false,
             "head_bias": true,
             "slimmable": {"method": "slice_channels_uniform",
                           "kwargs": {"allowed_channels": [1, 2]}}}
        ]
    })
}

/// Packed full-size weight count of `two_array_slim_config` (hand-computed:
/// array0 rechannel 3 + conv 9+3 + mixin 3 + layer1x1 9+3 + head 6+2 = 38;
/// array1 rechannel 6 + conv 4+2 + mixin 2 + layer1x1 4+2 + head 2+1 = 23;
/// + head_scale 1).
const TWO_ARRAY_FULL_LEN: usize = 62;

// -- Size -> channel mapping --------------------------------------------------

#[test]
fn channels_for_size_maps_ratio_through_allowed_channels() {
    let cfg = parse_full_wavenet_config(&json!({
        "layers": [
            {"input_size": 1, "condition_size": 1, "head_size": 1, "channels": 3,
             "dilations": [1], "kernel_size": 1, "activation": "ReLU", "gated": false,
             "head_bias": true,
             "slimmable": {"method": "slice_channels_uniform",
                           "kwargs": {"allowed_channels": [1, 2, 3]}}},
            // Non-slimmable array: keeps its full channel count at any size.
            {"input_size": 3, "condition_size": 1, "head_size": 1, "channels": 2,
             "dilations": [1], "kernel_size": 1, "activation": "ReLU", "gated": false,
             "head_bias": true}
        ]
    }))
    .unwrap();

    // Reference ratio_to_channels: idx = min(floor(ratio * len), len - 1).
    assert_eq!(channels_for_size(&cfg.layer_arrays, 0.0), vec![1, 2]);
    assert_eq!(channels_for_size(&cfg.layer_arrays, 0.33), vec![1, 2]);
    assert_eq!(channels_for_size(&cfg.layer_arrays, 0.34), vec![2, 2]);
    assert_eq!(channels_for_size(&cfg.layer_arrays, 0.99), vec![3, 2]);
    assert_eq!(channels_for_size(&cfg.layer_arrays, FULL_SIZE), vec![3, 2]);

    assert!(is_full_size(&cfg.layer_arrays, &[3, 2]));
    assert!(!is_full_size(&cfg.layer_arrays, &[1, 2]));
}

// -- Extraction walk pins -----------------------------------------------------

#[test]
fn full_slice_extraction_is_the_identity() {
    let cfg = parse_full_wavenet_config(&two_array_slim_config()).unwrap();
    let full = seq_weights(TWO_ARRAY_FULL_LEN);
    let extracted = extract_slimmed_weights(&cfg.layer_arrays, &full, &[3, 2]).unwrap();
    assert_eq!(extracted, full, "full-size slice must be the packed vector");
}

#[test]
fn smaller_slice_two_array_pin() {
    // Hand-computed against the packed layout of `two_array_slim_config`
    // with weights 0..62 and target channels [1, 1] (slim bottleneck 1 per
    // array). Extracting the wrong block or scaling the wrong dim visibly
    // misaligns these indices:
    //   array0: rechannel[0]; conv[3] bias[12]; mixin[15]; layer1x1[18]
    //           bias[27]; head_rechannel[30] (row 0 of 2 = next array's
    //           target, col 0 of 3 = slim bottleneck) bias[36]
    //   array1: rechannel[38] (col 0 of 3 = prev array's target); conv[44]
    //           bias[48]; mixin[50]; layer1x1[52] bias[56];
    //           head_rechannel[58] bias[60]
    //   head_scale[61]
    let cfg = parse_full_wavenet_config(&two_array_slim_config()).unwrap();
    let full = seq_weights(TWO_ARRAY_FULL_LEN);
    let extracted = extract_slimmed_weights(&cfg.layer_arrays, &full, &[1, 1]).unwrap();
    let expected: Vec<f32> = [
        0, 3, 12, 15, 18, 27, 30, 36, // array 0
        38, 44, 48, 50, 52, 56, 58, 60, // array 1
        61, // head_scale
    ]
    .into_iter()
    .map(|i: usize| i as f32)
    .collect();
    assert_eq!(extracted, expected);
}

#[test]
fn smaller_slice_gated_pin() {
    // Single gated array (channels 2, bottleneck 2, so the conv/mixin width
    // is doubled: B_g = 4 full, 2 slim). Weights 0..28; target [1].
    let cfg = parse_full_wavenet_config(&json!({
        "layers": [
            {"input_size": 1, "condition_size": 1, "head_size": 1, "channels": 2,
             "dilations": [1], "kernel_size": 1, "activation": "Tanh", "gated": true,
             "head_bias": true,
             "slimmable": {"method": "slice_channels_uniform",
                           "kwargs": {"allowed_channels": [1, 2]}}}
        ]
    }))
    .unwrap();
    // rechannel 2; conv 4x2 = 8, bias 4; mixin 4; layer1x1 4, bias 2;
    // head_rechannel 2, bias 1; head_scale 1 -> 28 weights.
    let full = seq_weights(28);
    let extracted = extract_slimmed_weights(&cfg.layer_arrays, &full, &[1]).unwrap();
    // Slim keeps conv rows 0..2 of 4 (primary + secondary halves both slim
    // to the bottleneck) and column 0 of 2.
    let expected: Vec<f32> = [0, 2, 4, 10, 11, 14, 15, 18, 22, 24, 26, 27]
        .into_iter()
        .map(|i: usize| i as f32)
        .collect();
    assert_eq!(extracted, expected);
}

#[test]
fn smaller_slice_film_pin() {
    // FiLM sites: conv_pre scales with channels (rows slice), while
    // input_mixin_pre is condition-sized and copies through unchanged.
    let cfg = parse_full_wavenet_config(&json!({
        "layers": [
            {"input_size": 1, "condition_size": 1, "head_size": 1, "channels": 2,
             "dilations": [1], "kernel_size": 1, "activation": "ReLU", "gated": false,
             "head_bias": false,
             "conv_pre_film": {},
             "input_mixin_pre_film": {"shift": false},
             "slimmable": {"method": "slice_channels_uniform",
                           "kwargs": {"allowed_channels": [1, 2]}}}
        ]
    }))
    .unwrap();
    // rechannel 2; conv 4, bias 2; mixin 2; layer1x1 4, bias 2;
    // conv_pre_film (shift: 2*ch = 4 rows) 4, bias 4; input_mixin_pre_film
    // (scale-only, cond 1) 1 + 1; head_rechannel 2 (no bias); head_scale 1
    // -> 29 weights.
    let full = seq_weights(29);
    let extracted = extract_slimmed_weights(&cfg.layer_arrays, &full, &[1]).unwrap();
    let expected: Vec<f32> = [0, 2, 6, 8, 10, 14, 16, 17, 20, 21, 24, 25, 26, 28]
        .into_iter()
        .map(|i: usize| i as f32)
        .collect();
    assert_eq!(extracted, expected);
}

#[test]
fn extraction_rejects_wrong_length_vectors() {
    let cfg = parse_full_wavenet_config(&two_array_slim_config()).unwrap();

    let short = seq_weights(TWO_ARRAY_FULL_LEN - 1);
    let err = extract_slimmed_weights(&cfg.layer_arrays, &short, &[1, 1]).unwrap_err();
    assert!(err.contains("packed weights exhausted"), "got: {err}");

    let long = seq_weights(TWO_ARRAY_FULL_LEN + 3);
    let err = extract_slimmed_weights(&cfg.layer_arrays, &long, &[1, 1]).unwrap_err();
    assert!(
        err.contains("3 unused weights after the slice extraction walk"),
        "got: {err}"
    );

    // Target vector inconsistencies.
    let full = seq_weights(TWO_ARRAY_FULL_LEN);
    let err = extract_slimmed_weights(&cfg.layer_arrays, &full, &[1]).unwrap_err();
    assert!(err.contains("must match the number of layer arrays"), "got: {err}");
    let err = extract_slimmed_weights(&cfg.layer_arrays, &full, &[1, 7]).unwrap_err();
    assert!(err.contains("target channel count"), "got: {err}");
}

#[test]
fn slicing_rejects_windowed_heads_and_grouped_convs() {
    // A windowed head rechannel (kernel_size > 1) cannot be sliced
    // (reference guard) — but only slicing hits the guard; full size loads.
    let windowed = parse_full_wavenet_config(&json!({
        "layers": [
            {"input_size": 1, "condition_size": 1, "channels": 2,
             "head": {"out_channels": 1, "kernel_size": 2, "bias": false},
             "dilations": [1], "kernel_size": 1, "activation": "ReLU", "gated": false,
             "slimmable": {"method": "slice_channels_uniform",
                           "kwargs": {"allowed_channels": [1, 2]}}}
        ]
    }))
    .unwrap();
    let err = extract_slimmed_weights(&windowed.layer_arrays, &seq_weights(64), &[1]).unwrap_err();
    assert!(err.contains("head rechannel kernel_size must be 1"), "got: {err}");

    let grouped = parse_full_wavenet_config(&json!({
        "layers": [
            {"input_size": 1, "condition_size": 1, "head_size": 1, "channels": 2,
             "groups_input": 2,
             "dilations": [1], "kernel_size": 1, "activation": "ReLU", "gated": false,
             "head_bias": true,
             "slimmable": {"method": "slice_channels_uniform",
                           "kwargs": {"allowed_channels": [1, 2]}}}
        ]
    }))
    .unwrap();
    let err = extract_slimmed_weights(&grouped.layer_arrays, &seq_weights(64), &[1]).unwrap_err();
    assert!(err.contains("groups_input > 1 not supported"), "got: {err}");
}

// -- Sliced-config derivation -------------------------------------------------

#[test]
fn derived_config_full_size_is_the_identity() {
    let cfg = parse_full_wavenet_config(&two_array_slim_config()).unwrap();
    let derived = derive_params_for_channels(&cfg.layer_arrays, &[3, 2]).unwrap();
    assert_eq!(derived, cfg.layer_arrays);
}

#[test]
fn derived_config_scales_only_the_channel_dependent_dims() {
    let cfg = parse_full_wavenet_config(&two_array_slim_config()).unwrap();
    let derived = derive_params_for_channels(&cfg.layer_arrays, &[1, 1]).unwrap();

    // Array 0: channels + bottleneck slim; input_size (first array) and
    // condition_size stay; head_size follows the NEXT array's target.
    assert_eq!(derived[0].channels, 1);
    assert_eq!(derived[0].bottleneck, 1);
    assert_eq!(derived[0].input_size, 1);
    assert_eq!(derived[0].condition_size, 1);
    assert_eq!(derived[0].head_size, 1);
    // Array 1: input_size follows the PREVIOUS array's target; head_size
    // (last array) stays.
    assert_eq!(derived[1].channels, 1);
    assert_eq!(derived[1].bottleneck, 1);
    assert_eq!(derived[1].input_size, 1);
    assert_eq!(derived[1].head_size, 1);
    // Everything else is untouched.
    assert_eq!(derived[0].dilations, cfg.layer_arrays[0].dilations);
    assert_eq!(derived[0].kernel_sizes, cfg.layer_arrays[0].kernel_sizes);
    assert_eq!(derived[0].activations, cfg.layer_arrays[0].activations);
    assert_eq!(derived[0].head_bias, cfg.layer_arrays[0].head_bias);
}

#[test]
fn derived_bottleneck_scales_proportionally_with_floor_one() {
    // layer1x1 active: bottleneck scales as max(1, bn * new / full).
    let cfg = parse_full_wavenet_config(&json!({
        "layers": [
            {"input_size": 1, "condition_size": 1, "head_size": 1, "channels": 4,
             "bottleneck": 2,
             "dilations": [1], "kernel_size": 1, "activation": "ReLU", "gated": false,
             "head_bias": true,
             "slimmable": {"method": "slice_channels_uniform",
                           "kwargs": {"allowed_channels": [1, 2, 4]}}}
        ]
    }))
    .unwrap();
    assert_eq!(derive_params_for_channels(&cfg.layer_arrays, &[2]).unwrap()[0].bottleneck, 1);
    assert_eq!(derive_params_for_channels(&cfg.layer_arrays, &[1]).unwrap()[0].bottleneck, 1);
    assert_eq!(derive_params_for_channels(&cfg.layer_arrays, &[4]).unwrap()[0].bottleneck, 2);

    // layer1x1 inactive: the bottleneck must track the channels.
    let cfg = parse_full_wavenet_config(&json!({
        "layers": [
            {"input_size": 1, "condition_size": 1, "head_size": 1, "channels": 4,
             "layer1x1": {"active": false, "groups": 1},
             "dilations": [1], "kernel_size": 1, "activation": "ReLU", "gated": false,
             "head_bias": true,
             "slimmable": {"method": "slice_channels_uniform",
                           "kwargs": {"allowed_channels": [2, 4]}}}
        ]
    }))
    .unwrap();
    assert_eq!(derive_params_for_channels(&cfg.layer_arrays, &[2]).unwrap()[0].bottleneck, 2);
}

// -- Real fixtures: full-slice loading ----------------------------------------

#[test]
fn slimmable_wavenet_fixture_loads_the_full_slice() {
    let loaded = load_model_from_file(&fixture_path("slimmable_wavenet.nam"))
        .expect("slimmable_wavenet.nam should load at full size");
    assert_eq!(loaded.sample_rate, 48_000.0);
    let mut model = loaded.model;
    for n in 0..64 {
        let y = model.process_sample(((n as f32) * 0.01).sin() * 0.25);
        assert!(y.is_finite(), "sample {n} not finite: {y}");
    }
}

#[test]
fn slimmable_fixture_matches_non_slimmable_twin_bit_for_bit() {
    // Full-slice passthrough: the packed vector IS the full-size layout, so
    // loading the slimmable file must equal loading the same file with the
    // slimmable descriptor stripped (the historical non-slimmable path).
    let mut file = fixture_json("slimmable_wavenet.nam");
    let mut slimmable_model = load_model_from_file(&fixture_path("slimmable_wavenet.nam"))
        .expect("fixture loads")
        .model;

    file["config"]["layers"][0]
        .as_object_mut()
        .unwrap()
        .remove("slimmable");
    let mut twin = load_value("stripped_twin", &file).expect("stripped twin loads");

    assert_outputs_bit_identical(
        slimmable_model.as_mut(),
        twin.as_mut(),
        256,
        "slimmable vs stripped twin",
    );
}

#[test]
fn a2_container_fixture_loads_the_full_submodel() {
    let loaded =
        load_model_from_file(&fixture_path("A2.nam")).expect("A2.nam container should load");
    assert_eq!(loaded.sample_rate, 48_000.0);
    let mut container_model = loaded.model;

    // The container's full size selects the LAST submodel (max_value 1.0,
    // the 8-channel "Full" WaveNet); loading that submodel standalone must
    // be bit-identical.
    let file = fixture_json("A2.nam");
    let full_submodel = file["config"]["submodels"][1]["model"].clone();
    assert_eq!(full_submodel["config"]["layers"][0]["channels"], 8);
    let mut standalone = load_value("a2_full_submodel", &full_submodel)
        .expect("standalone Full submodel loads");

    assert_outputs_bit_identical(
        container_model.as_mut(),
        standalone.as_mut(),
        256,
        "container vs standalone Full submodel",
    );
}

#[test]
fn non_slimmable_a2_fixtures_keep_loading() {
    // The slimmable probe must not misfire on plain A2 files (including
    // `slimmable: null`, which counts as absent).
    for name in ["wavenet_a2_max.nam", "wavenet_condition_dsp.nam"] {
        load_model_from_file(&fixture_path(name))
            .unwrap_or_else(|e| panic!("{name} should still load: {e}"));
    }
}

// -- Exact weight consumption -------------------------------------------------

#[test]
fn slimmable_fixture_weight_length_mismatch_is_rejected() {
    // One extra packed weight: the full-size layout no longer matches.
    let mut file = fixture_json("slimmable_wavenet.nam");
    file["weights"].as_array_mut().unwrap().push(json!(0.0));
    let err = load_err("extra_weight", &file);
    assert!(
        err.contains("1 unused weights after the slice extraction walk"),
        "got: {err}"
    );

    // One weight short: the layout verification runs out mid-walk (this
    // must NOT fall through to the engine's lenient head_scale default).
    let mut file = fixture_json("slimmable_wavenet.nam");
    file["weights"].as_array_mut().unwrap().pop();
    let err = load_err("short_weight", &file);
    assert!(err.contains("packed weights exhausted"), "got: {err}");
}

#[test]
fn container_submodel_weight_length_mismatch_is_rejected() {
    // Container submodels are strict: the Full submodel must consume its
    // own weight vector exactly.
    let mut file = fixture_json("A2.nam");
    file["config"]["submodels"][1]["model"]["weights"]
        .as_array_mut()
        .unwrap()
        .push(json!(0.0));
    let err = load_err("container_extra_weight", &file);
    assert!(err.contains("unused weights"), "got: {err}");
}

// -- Validation errors --------------------------------------------------------

#[test]
fn unknown_slimmable_method_is_rejected_at_load() {
    let mut file = fixture_json("slimmable_wavenet.nam");
    file["config"]["layers"][0]["slimmable"]["method"] = json!("prune_magnitude");
    let err = load_err("unknown_method", &file);
    assert!(
        err.contains("unsupported slimmable method 'prune_magnitude'"),
        "got: {err}"
    );
}

#[test]
fn slimmable_descriptor_inconsistencies_are_rejected_at_load() {
    // allowed_channels not strictly ascending.
    let mut file = fixture_json("slimmable_wavenet.nam");
    file["config"]["layers"][0]["slimmable"]["kwargs"]["allowed_channels"] = json!([2, 1, 3]);
    let err = load_err("unsorted_allowed", &file);
    assert!(err.contains("sorted strictly ascending"), "got: {err}");

    // Last allowed entry must equal the array's full channel count.
    let mut file = fixture_json("slimmable_wavenet.nam");
    file["config"]["layers"][0]["slimmable"]["kwargs"]["allowed_channels"] = json!([1, 2]);
    let err = load_err("wrong_last_allowed", &file);
    assert!(
        err.contains("must equal the full channel count"),
        "got: {err}"
    );
}

#[test]
fn slimmable_with_post_stack_head_is_rejected() {
    // Reference SlimmableWavenet: a post-stack head is not supported.
    let file = json!({
        "architecture": "WaveNet",
        "sample_rate": 48000,
        "config": {
            "layers": [
                {"input_size": 1, "condition_size": 1, "head_size": 1, "channels": 2,
                 "dilations": [1], "kernel_size": 1, "activation": "ReLU", "gated": false,
                 "head_bias": true,
                 "slimmable": {"method": "slice_channels_uniform",
                               "kwargs": {"allowed_channels": [1, 2]}}}
            ],
            "head": {"channels": 2, "num_layers": 1, "out_channels": 1}
        },
        "weights": [0.0]
    });
    let err = load_err("post_stack_head", &file);
    assert!(
        err.contains("post-stack head is not supported"),
        "got: {err}"
    );
}

#[test]
fn container_shape_errors_are_rejected() {
    // Empty submodel list.
    let mut file = fixture_json("A2.nam");
    file["config"]["submodels"] = json!([]);
    let err = load_err("container_empty", &file);
    assert!(err.contains("non-empty"), "got: {err}");

    // max_value not ascending.
    let mut file = fixture_json("A2.nam");
    file["config"]["submodels"][0]["max_value"] = json!(1.0);
    let err = load_err("container_unsorted", &file);
    assert!(err.contains("ascending max_value"), "got: {err}");

    // Last max_value below 1.0 leaves the full size uncovered.
    let mut file = fixture_json("A2.nam");
    file["config"]["submodels"][0]["max_value"] = json!(0.25);
    file["config"]["submodels"][1]["max_value"] = json!(0.5);
    let err = load_err("container_uncovered", &file);
    assert!(err.contains("max_value must be >= 1.0"), "got: {err}");

    // Selected submodel rate must match the container's.
    let mut file = fixture_json("A2.nam");
    file["config"]["submodels"][1]["model"]["sample_rate"] = json!(44100);
    let err = load_err("container_rate_mismatch", &file);
    assert!(err.contains("sample rate"), "got: {err}");
}

// -- Hand-packed two-size file, end-to-end ------------------------------------

#[test]
fn hand_packed_two_size_file_loads_the_full_slice_end_to_end() {
    let weights: Vec<Value> = (0..TWO_ARRAY_FULL_LEN).map(|i| json!(i as f32)).collect();
    let file = json!({
        "architecture": "WaveNet",
        "sample_rate": 48000,
        "config": two_array_slim_config(),
        "weights": weights
    });
    let mut packed = load_value("hand_packed", &file).expect("hand-packed two-size file loads");

    // Its full slice must construct the exact same model as the equivalent
    // non-slimmable export.
    let mut twin_file = file.clone();
    for layer in twin_file["config"]["layers"].as_array_mut().unwrap() {
        layer.as_object_mut().unwrap().remove("slimmable");
    }
    let mut twin = load_value("hand_packed_twin", &twin_file).expect("twin loads");

    assert_outputs_bit_identical(packed.as_mut(), twin.as_mut(), 64, "hand-packed vs twin");
}

#[test]
fn windowed_head_slimmable_file_still_loads_at_full_size() {
    // Slicing a windowed head is unimplemented (guard above), but the
    // full-size selection never slices — such a file loads, exactly like
    // the reference's is_full_size fast path.
    // rechannel 2; conv 4 + bias 2; mixin 2; layer1x1 4 + bias 2;
    // head_rechannel 1*2*2 taps = 4 (no bias); head_scale 1 -> 21 weights.
    let weights: Vec<Value> = (0..21).map(|i| json!(i as f32)).collect();
    let file = json!({
        "architecture": "WaveNet",
        "sample_rate": 48000,
        "config": {
            "layers": [
                {"input_size": 1, "condition_size": 1, "channels": 2,
                 "head": {"out_channels": 1, "kernel_size": 2, "bias": false},
                 "dilations": [1], "kernel_size": 1, "activation": "ReLU", "gated": false,
                 "slimmable": {"method": "slice_channels_uniform",
                               "kwargs": {"allowed_channels": [1, 2]}}}
            ]
        },
        "weights": weights
    });
    let mut model = load_value("windowed_slimmable", &file)
        .expect("windowed-head slimmable file loads at full size");
    for n in 0..16 {
        assert!(model.process_sample(0.1).is_finite(), "sample {n}");
    }
}
