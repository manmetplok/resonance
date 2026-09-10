//! Hostile `.nam` input regression tests: a file with absurd structural
//! fields (huge hidden_size/channels, wrap-inducing weight-count products,
//! huge dilations, truncated weight arrays) must be rejected with a typed
//! parse error — never a panic, an overflow wraparound, or an
//! allocation-failure abort. Reachable in production via tone3000
//! downloads and hand-placed model files.
//!
//! Every hostile file here is constructed as JSON in the test; a pair of
//! small valid models pins that the bounds don't reject real files.

use serde_json::{json, Value};

use resonance_amp::nam::parse::load_model_from_file;

/// Write `file` to a temp path, load it, clean up, and return the result.
fn load(name: &str, file: &Value) -> Result<(), String> {
    let path = std::env::temp_dir().join(format!(
        "resonance_amp_nam_hostile_{}_{name}.nam",
        std::process::id()
    ));
    std::fs::write(&path, serde_json::to_string(file).unwrap()).unwrap();
    let result = load_model_from_file(path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);
    result.map(|_| ())
}

fn load_err(name: &str, file: &Value) -> String {
    load(name, file).expect_err("hostile file must be rejected")
}

/// Deterministic small filler weights.
fn counted_weights(n: usize) -> Vec<f32> {
    (0..n).map(|i| 0.01 + (i as f32) * 0.003).collect()
}

fn lstm_file(input_size: u64, hidden_size: u64, num_layers: u64, weights: &[f32]) -> Value {
    json!({
        "architecture": "LSTM",
        "sample_rate": 48000,
        "config": {
            "input_size": input_size,
            "hidden_size": hidden_size,
            "num_layers": num_layers
        },
        "weights": weights
    })
}

fn wavenet_file(config: Value, weights: &[f32]) -> Value {
    json!({
        "architecture": "WaveNet",
        "sample_rate": 48000,
        "config": config,
        "weights": weights
    })
}

/// A minimal valid new-format layer array (see nam_wavenet_bottleneck.rs
/// for the hand-computed 57-weight consumption of this shape).
fn small_wavenet_config() -> Value {
    json!({
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 3, "bottleneck": 2,
            "dilations": [1, 2], "kernel_size": 2,
            "activation": "Tanh", "gating_mode": "none",
            "head_bias": true
        }],
        "head": null,
        "head_scale": 0.02
    })
}

// -- Valid models still load ---------------------------------------------------

#[test]
fn small_valid_models_still_load() {
    load("valid_wavenet", &wavenet_file(small_wavenet_config(), &counted_weights(57)))
        .expect("small valid WaveNet must load");
    // LSTM, hidden 2, 1 layer: 4*2*(1+2) + 4*2 + 2 + 2 = 36, head 2 + 1 = 39.
    load("valid_lstm", &lstm_file(1, 2, 1, &counted_weights(39)))
        .expect("small valid LSTM must load");
}

// -- LSTM ---------------------------------------------------------------------

#[test]
fn lstm_huge_hidden_size_is_rejected() {
    let err = load_err("lstm_huge_hidden", &lstm_file(1, 1 << 40, 1, &counted_weights(64)));
    assert!(err.contains("exceed"), "got: {err}");
}

#[test]
fn lstm_wrap_inducing_hidden_size_is_rejected() {
    // 4 * hs * (1 + hs) wraps past usize for hs around 2^31; the release
    // build's wrapped product used to slip past the bounds check and panic
    // slicing weights[pos..pos+count] with end < start.
    let err = load_err("lstm_wrap_hidden", &lstm_file(1, 1 << 31, 1, &counted_weights(64)));
    assert!(err.contains("exceed"), "got: {err}");
}

#[test]
fn lstm_huge_num_layers_is_rejected() {
    // Used to feed Vec::with_capacity directly: alloc-failure abort.
    let err = load_err("lstm_huge_layers", &lstm_file(1, 2, 1 << 40, &counted_weights(64)));
    assert!(err.contains("exceed"), "got: {err}");
}

#[test]
fn lstm_truncated_weights_error() {
    let err = load_err("lstm_truncated", &lstm_file(1, 2, 1, &counted_weights(20)));
    assert!(err.contains("Weight underflow"), "got: {err}");
}

// -- WaveNet, new (layer-array) format ----------------------------------------

#[test]
fn wavenet_huge_channels_is_rejected() {
    let config = json!({
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 1u64 << 40,
            "dilations": [1], "kernel_size": 2,
            "activation": "Tanh", "gating_mode": "none",
            "head_bias": true
        }],
        "head": null,
        "head_scale": 0.02
    });
    let err = load_err("wn_huge_channels", &wavenet_file(config, &counted_weights(64)));
    assert!(err.contains("exceeds the"), "got: {err}");
}

#[test]
fn wavenet_wrap_inducing_channels_is_rejected() {
    // channels^2 (the rechannel weight count) wraps past usize; the
    // wrapped count used to pass the bounds check in release builds.
    let config = json!({
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 1u64 << 33,
            "dilations": [1], "kernel_size": 2,
            "activation": "Tanh", "gating_mode": "none",
            "head_bias": true
        }],
        "head": null,
        "head_scale": 0.02
    });
    let err = load_err("wn_wrap_channels", &wavenet_file(config, &counted_weights(64)));
    assert!(err.contains("exceeds the"), "got: {err}");
}

#[test]
fn wavenet_huge_dilation_is_rejected() {
    // The dilation sizes the state ring, not the weight stream, so it gets
    // its own hard cap; a huge value used to size a multi-TiB allocation.
    let config = json!({
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 3, "bottleneck": 2,
            "dilations": [1, 1u64 << 50], "kernel_size": 2,
            "activation": "Tanh", "gating_mode": "none",
            "head_bias": true
        }],
        "head": null,
        "head_scale": 0.02
    });
    let err = load_err("wn_huge_dilation", &wavenet_file(config, &counted_weights(57)));
    assert!(err.contains("state-buffer size"), "got: {err}");
}

#[test]
fn wavenet_huge_head_kernel_size_is_rejected() {
    let config = json!({
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 3, "bottleneck": 2,
            "dilations": [1], "kernel_size": 2,
            "activation": "Tanh", "gating_mode": "none",
            "head": {"out_channels": 1, "kernel_size": 1u64 << 40, "bias": true}
        }],
        "head": null,
        "head_scale": 0.02
    });
    let err = load_err("wn_huge_head_kernel", &wavenet_file(config, &counted_weights(64)));
    assert!(err.contains("exceeds the"), "got: {err}");
}

#[test]
fn wavenet_zero_kernel_size_is_rejected() {
    // A zero kernel used to underflow the ring-capacity arithmetic.
    let config = json!({
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 3, "bottleneck": 2,
            "dilations": [1], "kernel_size": 0,
            "activation": "Tanh", "gating_mode": "none",
            "head_bias": true
        }],
        "head": null,
        "head_scale": 0.02
    });
    let err = load_err("wn_zero_kernel", &wavenet_file(config, &counted_weights(64)));
    assert!(err.contains("kernel size must be >= 1"), "got: {err}");
}

#[test]
fn wavenet_truncated_weights_error() {
    let err = load_err(
        "wn_truncated",
        &wavenet_file(small_wavenet_config(), &counted_weights(30)),
    );
    assert!(err.contains("Weight underflow"), "got: {err}");
}

#[test]
fn head_mlp_huge_num_layers_is_rejected() {
    // The legacy head MLP's num_layers sizes an allocation at parse time.
    let config = json!({
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 8,
            "channels": 3,
            "dilations": [1], "kernel_size": 2,
            "activation": "Tanh",
            "head_bias": true
        }],
        "head": {"channels": 8, "num_layers": 1u64 << 40, "out_channels": 1},
        "head_scale": 0.02
    });
    let err = load_err("wn_huge_head_mlp", &wavenet_file(config, &counted_weights(64)));
    assert!(err.contains("supported maximum"), "got: {err}");
}

// -- WaveNet, old flat format --------------------------------------------------

fn old_format_config(channels: u64, layers: Vec<u64>) -> Value {
    json!({
        "input_size": 1, "condition_size": 1, "head_size": 1,
        "channels": channels,
        "layers": layers,
        "head": [],
        "activation": "Tanh",
        "gated": false,
        "head_bias": true
    })
}

#[test]
fn old_format_huge_layer_count_is_rejected() {
    // Layer counts become 1 << i dilations; a hostile count used to
    // overflow the shift.
    let err = load_err(
        "old_huge_layer_count",
        &wavenet_file(old_format_config(2, vec![500]), &counted_weights(64)),
    );
    assert!(err.contains("not supported"), "got: {err}");
}

#[test]
fn old_format_huge_channels_is_rejected() {
    let err = load_err(
        "old_huge_channels",
        &wavenet_file(old_format_config(1 << 40, vec![2]), &counted_weights(64)),
    );
    assert!(err.contains("exceeds the"), "got: {err}");
}

// -- Slimmable packed-weight walk ----------------------------------------------

#[test]
fn slimmable_overflowing_bottleneck_is_rejected() {
    // The slimmable layout walk runs before any other validation; a gated
    // layer doubles the bottleneck, which used to wrap for hostile values.
    let config = json!({
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 2, "bottleneck": u64::MAX,
            "dilations": [1], "kernel_size": 1,
            "activation": "Tanh", "gating_mode": "gated",
            "head_bias": true,
            "slimmable": {"method": "slice_channels_uniform",
                          "kwargs": {"allowed_channels": [1, 2]}}
        }],
        "head": null,
        "head_scale": 0.02
    });
    let err = load_err("slim_overflow_bn", &wavenet_file(config, &counted_weights(64)));
    assert!(err.contains("overflows"), "got: {err}");
}

#[test]
fn slimmable_huge_default_allowed_channels_is_rejected() {
    // A missing kwargs.allowed_channels implies 1..=channels; a hostile
    // channel count used to abort materializing that list at parse time.
    let config = json!({
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 1u64 << 40,
            "dilations": [1], "kernel_size": 1,
            "activation": "Tanh",
            "head_bias": true,
            "slimmable": {"method": "slice_channels_uniform"}
        }],
        "head": null,
        "head_scale": 0.02
    });
    let err = load_err("slim_huge_default", &wavenet_file(config, &counted_weights(64)));
    assert!(err.contains("allowed_channels"), "got: {err}");
}
