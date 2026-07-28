//! The A2-marker gate and the shared reference forward semantics (ba
//! todos #1113/#1116).
//!
//! Since #1116 EVERY WaveNet model runs the reference
//! (NeuralAmpModelerCore) forward-pass STRUCTURE — raw-input/condition_dsp
//! condition, unconditionally consumed learned rechannels, chained head
//! accumulators with last-array output, no skip pre-activation, identity
//! residual without a layer1x1 — a user-approved sound change for legacy
//! A1 files (the historical engine wiring matched no official
//! implementation; doc #258). The `config_has_a2_markers` gate now selects
//! only the ACTIVATION FLAVOR: pre-A2-surface files resolve Tanh/Sigmoid
//! to the fast approximations (the official plugin runs
//! `enable_fast_tanh()`), A2-marked files use the exact functions.
//!
//! Covers: the marker probe, flavor selection, structural equivalence
//! across the gate, the activation-flavor divergence, the construction
//! validations (applied to marked and unmarked configs alike), and a
//! hand-computed pin of the reference head CHAINING + raw-input condition
//! across two stacks (end-to-end parity against real reference renders
//! lives in nam_a2_reference_parity.rs / nam_a1_reference_parity.rs).

use serde_json::{json, Value};

use resonance_amp::nam::parse::{
    config_has_a2_markers, load_model_from_file, parse_wavenet_config,
};
use resonance_amp::nam::{fast_tanh, NamInference};

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
fn plain_a1_configs_have_no_markers_and_keep_fast_activations() {
    let cfg = json!({"layers": [a1_layer()], "head": null, "head_scale": 0.02});
    assert!(!config_has_a2_markers(&cfg));
    assert!(parse_wavenet_config(cfg).unwrap().fast_activations);

    // kernel_sizes and the gated boolean predate A2: NOT markers.
    let mut layer = a1_layer();
    layer["kernel_sizes"] = json!([3]);
    layer["gated"] = json!(true);
    layer.as_object_mut().unwrap().remove("kernel_size");
    let cfg = json!({"layers": [layer], "head": null, "head_scale": 0.02});
    assert!(!config_has_a2_markers(&cfg));
    assert!(parse_wavenet_config(cfg).unwrap().fast_activations);
}

#[test]
fn every_a2_marker_flips_to_exact_activations() {
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
    // A marked config parses to the exact activation flavor.
    let mut layer = a1_layer();
    layer["gating_mode"] = json!("none");
    let cfg = json!({"layers": [layer], "head": null, "head_scale": 0.02});
    assert!(!parse_wavenet_config(cfg).unwrap().fast_activations);

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

// -- Construction validations (marked and unmarked alike) ---------------------

/// Two-stack config helper; stack shapes chosen per the args. `marked`
/// splices an inert A2 marker (`gating_mode: "none"`) into every layer.
fn two_stack_cfg(ch1: usize, head0: usize, bottleneck1: usize, marked: bool) -> Value {
    let mut layers = json!([
        {"input_size": 1, "condition_size": 1, "head_size": head0,
         "channels": 2, "dilations": [1], "kernel_size": 1,
         "activation": "Tanh", "head_bias": false},
        {"input_size": 2, "condition_size": 1, "head_size": 1,
         "channels": ch1, "bottleneck": bottleneck1,
         "dilations": [1], "kernel_size": 1,
         "activation": "Tanh", "head_bias": true}
    ]);
    if marked {
        for layer in layers.as_array_mut().unwrap() {
            layer["gating_mode"] = json!("none");
        }
    }
    json!({"layers": layers, "head": null, "head_scale": 1.0})
}

#[test]
fn chain_width_mismatches_are_rejected() {
    // channels of stack 1 != head_size of stack 0 (reference WaveNet ctor).
    let err = load_err(
        "chan_mismatch",
        &nam_file(two_stack_cfg(3, 2, 3, true), &[0.0; 64]),
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
        &nam_file(two_stack_cfg(2, 2, 3, true), &[0.0; 64]),
        "skip width mismatch",
    );
    assert!(
        err.contains("head accumulator width (3) doesn't match head_size of preceding stack (2)"),
        "{err}"
    );

    // Gate equivalence: the SAME validation applies to a marker-less
    // config (all models share the reference structure since #1116; the
    // bottleneck field is dropped so the config stays pre-A2-surface —
    // channels 3 vs head_size 2 still mismatches).
    let mut cfg = two_stack_cfg(3, 2, 3, false);
    cfg["layers"][1].as_object_mut().unwrap().remove("bottleneck");
    assert!(!config_has_a2_markers(&cfg));
    let err = load_err(
        "chan_mismatch_unmarked",
        &nam_file(cfg, &[0.0; 64]),
        "channels/head_size mismatch (unmarked)",
    );
    assert!(
        err.contains("channels of stack 1 (3) doesn't match head_size of preceding stack (2)"),
        "{err}"
    );
}

#[test]
fn condition_width_and_input_width_are_validated() {
    // Applies to marker-less (A1-surface) configs too.
    let mut layer = a1_layer();
    layer["condition_size"] = json!(2);
    let cfg = json!({"layers": [layer], "head": null, "head_scale": 1.0});
    assert!(!config_has_a2_markers(&cfg));
    let err = load_err("cond_width", &nam_file(cfg, &[0.0; 32]), "condition width mismatch");
    assert!(
        err.contains("condition_size (2) must match the model input channels (1)"),
        "{err}"
    );

    let mut layer = a1_layer();
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
    let err = load_err("a2_head", &nam_file(cfg, &[0.0; 32]), "post-stack head on an A2-marked config");
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
fn chain_cfg(marked: bool) -> Value {
    let mut layers = json!([
        {"input_size": 1, "condition_size": 1, "head_size": 1,
         "channels": 1, "dilations": [1], "kernel_size": 1,
         "activation": "ReLU", "head_bias": false},
        {"input_size": 1, "condition_size": 1, "head_size": 1,
         "channels": 1, "dilations": [1], "kernel_size": 1,
         "activation": "ReLU", "head_bias": false}
    ]);
    if marked {
        for layer in layers.as_array_mut().unwrap() {
            layer["gating_mode"] = json!("none");
        }
    }
    json!({"layers": layers, "head": null, "head_scale": 1.0})
}

/// Per stack (1-wide, kernel 1): rechannel, conv w, conv bias, mixin,
/// layer1x1 w, layer1x1 b, head_rechannel.
const CHAIN_WEIGHTS: [f32; 15] = [
    2.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, // stack 0
    0.5, 1.0, 0.0, 1.0, 1.0, 0.0, 0.5, // stack 1
    1.0, // head_scale
];

#[test]
fn reference_head_chaining_and_raw_condition_hand_computed() {
    // The config carries NO A2 marker: since #1116 the pre-A2 surface runs
    // the same reference structure (this used to be the legacy path).
    let cfg = chain_cfg(false);
    assert!(!config_has_a2_markers(&cfg));
    let mut model =
        load_value("chain_pin", &nam_file(cfg, &CHAIN_WEIGHTS)).expect("chain model loads");

    for &x in &[0.125f32, 0.25, 0.5] {
        let out = model.process_sample(x);
        assert_eq!(
            out.to_bits(),
            (3.25 * x).to_bits(),
            "reference chain must yield 3.25x (got {out} for input {x})"
        );
    }
}

/// Gate equivalence: with a flavor-independent activation (ReLU) a marked
/// and an unmarked twin of the same weights must be BIT-IDENTICAL — the
/// gate no longer selects structure, only the Tanh/Sigmoid flavor.
#[test]
fn marked_and_unmarked_relu_configs_are_bit_identical() {
    let mut unmarked = load_value("eq_unmarked", &nam_file(chain_cfg(false), &CHAIN_WEIGHTS))
        .expect("unmarked chain model loads");
    let mut marked = load_value("eq_marked", &nam_file(chain_cfg(true), &CHAIN_WEIGHTS))
        .expect("marked chain model loads");
    for i in 0..32 {
        let x = ((i as f32) * 0.29).sin() * 0.7;
        let a = unmarked.process_sample(x);
        let b = marked.process_sample(x);
        assert!(a.is_finite());
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "sample {i}: the gate must not change structure ({a} vs {b})"
        );
    }
}

// -- Activation-flavor divergence ---------------------------------------------

/// The gate's ONLY remaining effect: `Tanh` resolves to the fast tanh on
/// the pre-A2 surface and to the exact tanh on an A2-marked config —
/// pinned by hand on a single 1-wide layer where
/// out = hr * act(conv*(rc*x) + mixin*x) * scale.
#[test]
fn gate_selects_activation_flavor_only() {
    let layer = |marked: bool| {
        let mut l = json!({
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 1, "dilations": [1], "kernel_size": 1,
            "activation": "Tanh", "head_bias": false
        });
        if marked {
            l["gating_mode"] = json!("none");
        }
        json!({"layers": [l], "head": null, "head_scale": 1.0})
    };
    // rechannel, conv w, conv b, mixin, layer1x1 w, layer1x1 b,
    // head_rechannel, head_scale.
    let (rc, cw, cb, m, hr, s) = (0.5f32, 1.5f32, 0.0f32, 0.25f32, 2.0f32, 1.0f32);
    let weights = [rc, cw, cb, m, 1.0, 0.0, hr, s];

    let mut fast = load_value("flavor_fast", &nam_file(layer(false), &weights))
        .expect("unmarked model loads");
    let mut exact = load_value("flavor_exact", &nam_file(layer(true), &weights))
        .expect("marked model loads");

    let mut differed = false;
    for &x in &[0.25f32, -0.5, 0.75, 1.5] {
        let z = (cw * (rc * x)) + cb + m * x;
        let out_fast = fast.process_sample(x);
        let out_exact = exact.process_sample(x);
        assert_eq!(
            out_fast.to_bits(),
            ((hr * fast_tanh(z)) * s).to_bits(),
            "pre-A2 surface must use the fast tanh"
        );
        assert_eq!(
            out_exact.to_bits(),
            ((hr * z.tanh()) * s).to_bits(),
            "A2-marked config must use the exact tanh"
        );
        differed |= out_fast.to_bits() != out_exact.to_bits();
    }
    assert!(
        differed,
        "fast and exact tanh must diverge somewhere on these inputs"
    );
}
