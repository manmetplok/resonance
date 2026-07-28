//! The legacy/reference semantic gate (ba todo #1113).
//!
//! A WaveNet config runs with REFERENCE (NeuralAmpModelerCore) forward
//! semantics iff it carries any A2 marker (`config_has_a2_markers`);
//! configs fully expressible in the pre-A2 surface keep the historical
//! engine semantics bit-for-bit, so existing A1 projects keep their sound.
//! Covers the marker probe, the reference-only construction validations,
//! and a hand-computed pin of the reference head CHAINING + raw-input
//! condition across two stacks (the wiring divergences reconciled in
//! #1113; end-to-end parity against real reference renders lives in
//! nam_a2_reference_parity.rs).

use serde_json::{json, Value};

use resonance_amp::nam::parse::{
    config_has_a2_markers, load_model_from_file, parse_wavenet_config,
};
use resonance_amp::nam::NamInference;

fn write_temp_nam(name: &str, file: &Value) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "resonance_amp_nam_refsem_{}_{name}.nam",
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

/// Load expected to fail; returns the error message.
fn load_err(name: &str, file: &Value, what: &str) -> String {
    match load_value(name, file) {
        Ok(_) => panic!("{what}: expected the load to fail"),
        Err(e) => e,
    }
}

fn nam_file(config: Value, weights: &[f32]) -> Value {
    json!({
        "architecture": "WaveNet",
        "sample_rate": 48000,
        "config": config,
        "weights": weights
    })
}

/// The plain A1 layer-array surface: no A2 markers.
fn a1_layer() -> Value {
    json!({
        "input_size": 1, "condition_size": 1, "head_size": 1,
        "channels": 2, "dilations": [1], "kernel_size": 1,
        "activation": "Tanh", "gated": false, "head_bias": true
    })
}

// -- Marker probe -------------------------------------------------------------

#[test]
fn plain_a1_configs_have_no_markers_and_stay_legacy() {
    let cfg = json!({"layers": [a1_layer()], "head": null, "head_scale": 0.02});
    assert!(!config_has_a2_markers(&cfg));
    assert!(!parse_wavenet_config(cfg).unwrap().reference_semantics);

    // kernel_sizes and the gated boolean predate A2: NOT markers.
    let mut layer = a1_layer();
    layer["kernel_sizes"] = json!([3]);
    layer["gated"] = json!(true);
    layer.as_object_mut().unwrap().remove("kernel_size");
    let cfg = json!({"layers": [layer], "head": null, "head_scale": 0.02});
    assert!(!config_has_a2_markers(&cfg));
    assert!(!parse_wavenet_config(cfg).unwrap().reference_semantics);
}

#[test]
fn every_a2_marker_flips_to_reference_semantics() {
    let layer_markers: [(&str, Value); 12] = [
        ("bottleneck", json!(2)),
        ("gating_mode", json!("none")),
        ("secondary_activation", json!("Sigmoid")),
        ("groups_input", json!(1)),
        ("groups_input_mixin", json!(1)),
        ("layer1x1", json!({"active": true, "groups": 1})),
        ("head1x1", json!({"active": false, "out_channels": 2, "groups": 1})),
        ("head", json!({"out_channels": 1, "kernel_size": 1, "bias": true})),
        ("slimmable", json!({"method": "slice_channels_uniform", "kwargs": {"allowed_channels": [1, 2]}})),
        ("activation", json!({"type": "LeakyReLU", "negative_slope": 0.01})),
        ("activation", json!(["Tanh"])),
        ("conv_pre_film", json!(false)),
    ];
    for (key, value) in layer_markers {
        let mut layer = a1_layer();
        layer[key] = value.clone();
        let cfg = json!({"layers": [layer], "head": null, "head_scale": 0.02});
        assert!(
            config_has_a2_markers(&cfg),
            "layer key {key} = {value} must be an A2 marker"
        );
    }

    // Top-level markers: condition_dsp, and an A2-style post-stack head.
    let cfg = json!({
        "layers": [a1_layer()],
        "head": null, "head_scale": 0.02,
        "condition_dsp": {"architecture": "WaveNet", "config": {}, "weights": []}
    });
    assert!(config_has_a2_markers(&cfg));
    let cfg = json!({
        "layers": [a1_layer()],
        "head": {"channels": 2, "out_channels": 1, "kernel_sizes": [1], "activation": "Tanh"},
        "head_scale": 0.02
    });
    assert!(config_has_a2_markers(&cfg));
    // ... but null / legacy-MLP-shaped heads are not markers.
    let cfg = json!({
        "layers": [a1_layer()],
        "head": {"channels": 2, "num_layers": 1, "out_channels": 1},
        "head_scale": 0.02
    });
    assert!(!config_has_a2_markers(&cfg));
}

// -- Reference-only construction validations ----------------------------------

/// Two-stack reference config helper; stack shapes chosen per the args.
fn two_stack_cfg(ch1: usize, head0: usize, bottleneck1: usize) -> Value {
    json!({"layers": [
        {"input_size": 1, "condition_size": 1, "head_size": head0,
         "channels": 2, "dilations": [1], "kernel_size": 1,
         "activation": "Tanh", "gating_mode": "none", "head_bias": false},
        {"input_size": 2, "condition_size": 1, "head_size": 1,
         "channels": ch1, "bottleneck": bottleneck1,
         "dilations": [1], "kernel_size": 1,
         "activation": "Tanh", "gating_mode": "none", "head_bias": true}
    ], "head": null, "head_scale": 1.0})
}

#[test]
fn reference_chain_width_mismatches_are_rejected() {
    // channels of stack 1 != head_size of stack 0 (reference WaveNet ctor).
    let err = load_err(
        "chan_mismatch",
        &nam_file(two_stack_cfg(3, 2, 3), &[0.0; 64]),
        "channels/head_size mismatch",
    );
    assert!(
        err.contains("channels of stack 1 (3) doesn't match head_size of preceding stack (2)"),
        "{err}"
    );

    // Skip accumulator width (bottleneck) != preceding head_size: the
    // head-chain seed would be misaligned (the reference memcpy assumes
    // equality). channels matches the preceding head_size so the first
    // check passes; the diverging bottleneck (with its active layer1x1)
    // sets the skip width.
    let err = load_err(
        "skip_mismatch",
        &nam_file(two_stack_cfg(2, 2, 3), &[0.0; 64]),
        "skip width mismatch",
    );
    assert!(
        err.contains("head accumulator width (3) doesn't match head_size of preceding stack (2)"),
        "{err}"
    );
}

#[test]
fn reference_condition_width_and_input_width_are_validated() {
    let mut layer = a1_layer();
    layer["gating_mode"] = json!("none");
    layer["condition_size"] = json!(2);
    let cfg = json!({"layers": [layer], "head": null, "head_scale": 1.0});
    let err = load_err("cond_width", &nam_file(cfg, &[0.0; 32]), "condition width mismatch");
    assert!(
        err.contains("condition_size (2) must match the model input channels (1)"),
        "{err}"
    );

    let mut layer = a1_layer();
    layer["gating_mode"] = json!("none");
    layer["input_size"] = json!(2);
    layer["condition_size"] = json!(2);
    let cfg = json!({"layers": [layer], "head": null, "head_scale": 1.0});
    let err = load_err("multi_in", &nam_file(cfg, &[0.0; 32]), "multi-channel input");
    assert!(err.contains("only mono models are supported"), "{err}");
}

#[test]
fn a2_post_stack_head_is_rejected_with_a_clear_error() {
    let mut layer = a1_layer();
    layer["gating_mode"] = json!("none");
    let cfg = json!({
        "layers": [layer],
        "head": {"channels": 2, "num_layers": 1, "out_channels": 1},
        "head_scale": 1.0
    });
    let err = load_err("a2_head", &nam_file(cfg, &[0.0; 32]), "post-stack head under reference semantics");
    assert!(
        err.contains("a post-stack 'head' is not supported for A2 models"),
        "{err}"
    );
}

// -- Hand-computed reference chaining + raw-input condition -------------------

/// Two chained 1-wide stacks, kernel 1, ReLU, all-positive weights: every
/// ReLU is the identity for x > 0 and the arithmetic is exact in f32
/// (powers-of-two inputs, dyadic-rational weights).
///
/// The reference LayerArray ctor constructs its input rechannel
/// UNCONDITIONALLY — a 1-to-1 rechannel is a learned conv, not an identity
/// — so each 1-wide stack consumes 7 weights (rechannel, conv w, conv b,
/// mixin, layer1x1 w, layer1x1 b, head_rechannel): 15 total with the head
/// scale. The rechannel weights are deliberately != 1 so this pins their
/// consumption *and* their arithmetic.
///
/// Reference semantics (all values for input x > 0):
///   condition       = raw input x (every stack)
///   stack 0: a0     = 2 * x                           = 2x   (rechannel)
///            z0     = relu(1*a0 + 0 + 1*x)            = 3x
///            hr0    = 1 * z0                          = 3x   (head chain)
///            out0   = a0 + (1*z0 + 0)                 = 5x   (audio chain)
///   stack 1: a1     = 0.5 * out0                      = 2.5x (rechannel)
///            z1     = relu(1*a1 + 0 + 1*x)            = 3.5x
///            skip1  = hr0 + z1                        = 6.5x (seeded!)
///            hr1    = 0.5 * skip1                     = 3.25x
///   output          = hr1 * head_scale(1)             = 3.25x
///
/// The legacy engine wires this differently on every count (skipped 1-to-1
/// rechannels, zeroed skip accumulators, summed head outputs,
/// post-rechannel condition), so this pins all the reconciled divergences
/// at once.
#[test]
fn reference_head_chaining_and_raw_condition_hand_computed() {
    let cfg = json!({"layers": [
        {"input_size": 1, "condition_size": 1, "head_size": 1,
         "channels": 1, "dilations": [1], "kernel_size": 1,
         "activation": "ReLU", "gating_mode": "none", "head_bias": false},
        {"input_size": 1, "condition_size": 1, "head_size": 1,
         "channels": 1, "dilations": [1], "kernel_size": 1,
         "activation": "ReLU", "gating_mode": "none", "head_bias": false}
    ], "head": null, "head_scale": 1.0});
    // Per stack (1-wide, kernel 1): rechannel, conv w, conv bias, mixin,
    // layer1x1 w, layer1x1 b, head_rechannel.
    let weights = [
        2.0f32, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, // stack 0
        0.5, 1.0, 0.0, 1.0, 1.0, 0.0, 0.5, // stack 1
        1.0, // head_scale
    ];
    let mut model =
        load_value("chain_pin", &nam_file(cfg, &weights)).expect("chain model loads");

    for &x in &[0.125f32, 0.25, 0.5] {
        let out = model.process_sample(x);
        assert_eq!(
            out.to_bits(),
            (3.25 * x).to_bits(),
            "reference chain must yield 3.25x (got {out} for input {x})"
        );
    }
}
