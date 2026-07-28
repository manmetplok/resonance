//! Tests for the A2 bottleneck channel count in WaveNet construction:
//! weight consumption matching the NeuralAmpModelerCore reference formula,
//! weight-layout correctness against hand-computed inference, A1 degeneracy
//! (bottleneck == channels is bit-identical to the historical layout), and
//! construction-time validation of degenerate configs.

use resonance_amp::nam::activations::{ActivationConfig, ActivationKind};
use resonance_amp::nam::parse::{load_model_from_file, StackConfig, WaveNetConfig, WeightReader};
use resonance_amp::nam::wavenet::params::{GatingMode, Head1x1Params};
use resonance_amp::nam::wavenet::WaveNetModel;
use resonance_amp::nam::{fast_tanh, NamInference};

fn write_temp_nam(name: &str, body: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "resonance_amp_nam_bottleneck_{}_{name}.nam",
        std::process::id()
    ));
    std::fs::write(&path, body).unwrap();
    path
}

fn load_nam(name: &str, body: &str) -> Result<Box<dyn NamInference>, String> {
    let path = write_temp_nam(name, body);
    let result = load_model_from_file(path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);
    result.map(|loaded| loaded.model)
}

/// Deterministic small filler weights (values are irrelevant for the
/// consumption-count tests; kept small to avoid activation saturation).
fn counted_weights(n: usize) -> Vec<f32> {
    (0..n).map(|i| 0.01 + (i as f32) * 0.003).collect()
}

fn new_format_json(config: &str, weights: &[f32]) -> String {
    let ws: Vec<String> = weights.iter().map(|w| w.to_string()).collect();
    format!(
        r#"{{
            "architecture": "WaveNet",
            "sample_rate": 48000,
            "config": {config},
            "weights": [{}]
        }}"#,
        ws.join(",")
    )
}

// -- Weight consumption vs the reference formula -----------------------------

/// Non-gated, channels=3, bottleneck=2, 2 layers (kernel 2, condition 1),
/// new-format (layer1x1 active, head_bias true). Reference consumption
/// (NAM/wavenet/model.cpp set_weights_ order):
///   rechannel        3*1        =  3
///   per layer: conv  2*3*2 = 12, bias 2, mixin 2*1 = 2,
///              layer1x1 3*2 = 6 + 3     -> 25 each, 2 layers = 50
///   head_rechannel   1*2 + 1(bias)      =  3
///   head_scale                          =  1
///   total                               = 57
const NONGATED_CONFIG: &str = r#"{
    "layers": [{
        "input_size": 1, "condition_size": 1, "head_size": 1,
        "channels": 3, "bottleneck": 2,
        "dilations": [1, 2], "kernel_size": 2,
        "activation": "Tanh", "gating_mode": "none",
        "head_bias": true
    }],
    "head": null,
    "head_scale": 0.02
}"#;
const NONGATED_TOTAL: usize = 57;

#[test]
fn nongated_bottleneck_consumes_reference_weight_count() {
    let weights = counted_weights(NONGATED_TOTAL);
    let mut model = load_nam("nongated_exact", &new_format_json(NONGATED_CONFIG, &weights))
        .expect("exact weight count must load");
    assert!(model.process_sample(0.1).is_finite());

    // Two weights short: construction must underflow before the trailing
    // (optional) head_scale read. The exact total is pinned by the
    // direct-construction tests below (remaining() == 0).
    let err = load_nam(
        "nongated_short",
        &new_format_json(NONGATED_CONFIG, &weights[..NONGATED_TOTAL - 2]),
    )
    .err()
    .expect("short weight vector must fail");
    assert!(err.contains("Weight underflow"), "{err}");
}

/// Gated variant: the conv and input mixin output 2*bottleneck channels.
/// channels=3, bottleneck=2, 1 layer (kernel 2, condition 1):
///   rechannel      3*1          =  3
///   conv           (2*2)*3*2    = 24, bias 4
///   mixin          (2*2)*1      =  4
///   layer1x1       3*2 + 3      =  9
///   head_rechannel 1*2 + 1      =  3
///   head_scale                  =  1
///   total                       = 48
const GATED_CONFIG: &str = r#"{
    "layers": [{
        "input_size": 1, "condition_size": 1, "head_size": 1,
        "channels": 3, "bottleneck": 2,
        "dilations": [1], "kernel_size": 2,
        "activation": "Tanh", "gating_mode": "gated",
        "head_bias": true
    }],
    "head": null,
    "head_scale": 0.02
}"#;
const GATED_TOTAL: usize = 48;

#[test]
fn gated_bottleneck_conv_consumes_two_bottleneck_output_channels() {
    let weights = counted_weights(GATED_TOTAL);
    let mut model = load_nam("gated_exact", &new_format_json(GATED_CONFIG, &weights))
        .expect("exact weight count must load");
    assert!(model.process_sample(0.1).is_finite());

    let err = load_nam(
        "gated_short",
        &new_format_json(GATED_CONFIG, &weights[..GATED_TOTAL - 2]),
    )
    .err()
    .expect("short weight vector must fail");
    assert!(err.contains("Weight underflow"), "{err}");
}

/// Same constructions driven directly through WeightReader: every weight is
/// consumed (head_scale included), nothing remains — this pins the exact
/// consumption totals to the reference formula.
fn bottleneck_stack(
    channels: usize,
    bottleneck: usize,
    dilations: Vec<usize>,
    gated: bool,
) -> StackConfig {
    let n = dilations.len();
    let (gating_modes, secondary_activations) = if gated {
        (
            vec![GatingMode::Gated; n],
            vec![Some(ActivationConfig::simple(ActivationKind::Sigmoid)); n],
        )
    } else {
        (vec![GatingMode::None; n], vec![None; n])
    };
    StackConfig {
        input_size: 1,
        condition_size: 1,
        head_size: 1,
        channels,
        bottleneck,
        dilations,
        kernel_sizes: vec![2; n],
        activation: ActivationConfig::from_name("Tanh").unwrap(),
        gating_modes,
        secondary_activations,
        groups_input: 1,
        groups_input_mixin: 1,
        layer1x1_groups: 1,
        head1x1: Head1x1Params::inactive(channels),
    }
}

#[test]
fn direct_construction_consumes_every_weight() {
    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![bottleneck_stack(3, 2, vec![1, 2], false)],
        head: vec![],
        head_size: 1,
        head_bias: true,
        has_layer1x1: true,
    };
    let weights = counted_weights(NONGATED_TOTAL);
    let mut reader = WeightReader::new(&weights);
    WaveNetModel::from_config_and_weights(config, &mut reader).expect("construction must succeed");
    assert_eq!(reader.remaining(), 0, "all weights must be consumed");
}

#[test]
fn direct_gated_construction_consumes_every_weight() {
    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![bottleneck_stack(3, 2, vec![1], true)],
        head: vec![],
        head_size: 1,
        head_bias: true,
        has_layer1x1: true,
    };
    let weights = counted_weights(GATED_TOTAL);
    let mut reader = WeightReader::new(&weights);
    WaveNetModel::from_config_and_weights(config, &mut reader).expect("construction must succeed");
    assert_eq!(reader.remaining(), 0, "all weights must be consumed");
}

// -- Weight layout: hand-computed inference with bottleneck != channels ------

/// channels=2, bottleneck=1, non-gated, single layer with kernel 1 so the
/// full computation is hand-checkable:
///   rechannel [r0, r1], conv [c0, c1], conv bias [b], mixin [m],
///   layer1x1 [l0, l1] + bias [p0, p1], head_rechannel [h] (no bias),
///   head_scale [s]                                    -> 12 weights
#[test]
fn bottleneck_layout_output_is_bit_identical_to_hand_computed_reference() {
    let config = r#"{
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 2, "bottleneck": 1,
            "dilations": [1], "kernel_size": 1,
            "activation": "Tanh", "gating_mode": "none",
            "head_bias": false
        }],
        "head": null,
        "head_scale": 2.0
    }"#;
    let weights: [f32; 12] = [
        0.3, -0.4, // rechannel r0, r1
        0.5, 0.25, // conv c0, c1
        0.05, // conv bias b
        0.6, // mixin m
        0.7, -0.2, // layer1x1 l0, l1
        0.01, -0.01, // layer1x1 bias p0, p1
        0.9, // head_rechannel h
        2.0, // head_scale s
    ];
    let [r0, r1, c0, c1, b, m, l0, l1, _p0, _p1, h, s] = weights;
    let mut model = load_nam("layout", &new_format_json(config, &weights))
        .expect("bottleneck model must load");

    for &x in &[0.25f32, -0.5, 0.75] {
        // Engine op order: rechannel, condition snapshot (channel 0),
        // conv matvec + bias + mixin, activation at bottleneck width,
        // skip pre-activation, head rechannel, head scale. Kernel size 1
        // means the layer has no memory, so each sample is independent.
        let a0 = r0 * x;
        let a1 = r1 * x;
        let z_pre = ((c0 * a0 + c1 * a1) + b) + m * a0;
        let z = fast_tanh(z_pre);
        let expected = (h * fast_tanh(z)) * s;
        let out = model.process_sample(x);
        assert_eq!(
            out.to_bits(),
            expected.to_bits(),
            "bottleneck output must be bit-identical (got {out}, expected {expected})"
        );
        // layer1x1 weights are consumed but only feed the (unused) residual
        // of this single-layer stack; reference the bindings so the
        // destructuring stays honest about the layout.
        let _ = (l0, l1);
    }
}

// -- A1 degeneracy: bottleneck == channels ------------------------------------

/// An explicit `bottleneck` equal to `channels` must construct the exact
/// same model as omitting the field (the A1 default): same weight
/// consumption, bit-identical output stream.
#[test]
fn explicit_bottleneck_equal_to_channels_is_bit_identical_to_a1_default() {
    let base = r#""input_size": 1, "condition_size": 1, "head_size": 1,
        "channels": 2,
        "dilations": [1, 2], "kernel_size": 2,
        "activation": "Tanh", "gating_mode": "gated",
        "head_bias": true"#;
    let with_field = format!(
        r#"{{"layers": [{{ {base}, "bottleneck": 2 }}], "head": null, "head_scale": 1.0}}"#
    );
    let without_field = format!(r#"{{"layers": [{{ {base} }}], "head": null, "head_scale": 1.0}}"#);

    // channels=2, bottleneck=2, gated, 2 layers, kernel 2:
    // rechannel 2 + 2*(conv 4*2*2=16 + bias 4 + mixin 4 + l1x1 4+2)
    // + head_rechannel 1*2+1=3 + head_scale 1 — the exact reference total,
    // so the load leaves no unused weights.
    let count = 2 + 2 * (16 + 4 + 4 + 6) + (2 + 1) + 1;
    let weights = counted_weights(count);

    let mut explicit = load_nam("bn_explicit", &new_format_json(&with_field, &weights))
        .expect("explicit bottleneck == channels must load");
    let mut default = load_nam("bn_default", &new_format_json(&without_field, &weights))
        .expect("A1 default must load");

    for i in 0..32 {
        let x = ((i as f32) * 0.37).sin() * 0.8;
        let a = explicit.process_sample(x);
        let b = default.process_sample(x);
        assert!(a.is_finite());
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "bottleneck == channels must degenerate to the A1 layout exactly"
        );
    }
}

// -- Inference smoke: shapes, receptive field, finite output ------------------

/// bottleneck != channels model with real dilation memory: output must be
/// finite everywhere and depend on exactly the receptive field
/// (sum of (kernel_size - 1) * dilation = 3 samples of history).
#[test]
fn bottleneck_inference_respects_receptive_field_and_stays_finite() {
    let weights = counted_weights(NONGATED_TOTAL);
    let json = new_format_json(NONGATED_CONFIG, &weights);
    let mut model_a = load_nam("rf_a", &json).unwrap();
    let mut model_b = load_nam("rf_b", &json).unwrap();

    // Streams differ only at sample 0.
    let n = 16;
    let input_a: Vec<f32> = (0..n).map(|i| ((i as f32) * 0.61).cos() * 0.5).collect();
    let mut input_b = input_a.clone();
    input_b[0] = -input_a[0] + 0.4;

    let out_a: Vec<f32> = input_a.iter().map(|&x| model_a.process_sample(x)).collect();
    let out_b: Vec<f32> = input_b.iter().map(|&x| model_b.process_sample(x)).collect();

    assert!(out_a.iter().all(|v| v.is_finite()));
    assert!(out_b.iter().all(|v| v.is_finite()));
    assert_ne!(
        out_a[0].to_bits(),
        out_b[0].to_bits(),
        "sample 0 depends on input 0"
    );
    // Receptive field is 3: from sample 4 on, input 0 is out of reach and
    // the outputs must agree exactly.
    for i in 4..n {
        assert_eq!(
            out_a[i].to_bits(),
            out_b[i].to_bits(),
            "sample {i} must not depend on input 0 (receptive field is 3)"
        );
    }
}

// -- Construction-time validation ---------------------------------------------

#[test]
fn zero_layer_configs_are_rejected_at_construction() {
    // Old-format config with an empty stack list.
    let empty_stacks = r#"{
        "architecture": "WaveNet",
        "config": {
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 1, "layers": [], "head": [],
            "activation": "Tanh", "gated": false, "head_bias": false
        },
        "weights": [0.0]
    }"#;
    let err = load_nam("empty_stacks", empty_stacks)
        .err()
        .expect("empty stack list must fail");
    assert!(err.contains("no layers"), "{err}");

    // Old-format config with a stack that has zero layers.
    let empty_stack = r#"{
        "architecture": "WaveNet",
        "config": {
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 1, "layers": [0], "head": [],
            "activation": "Tanh", "gated": false, "head_bias": false
        },
        "weights": [0.0]
    }"#;
    let err = load_nam("empty_stack", empty_stack)
        .err()
        .expect("zero-layer stack must fail");
    assert!(err.contains("no layers"), "{err}");
}

#[test]
fn bottleneck_without_layer1x1_is_rejected() {
    // Without a layer1x1 there is nothing to map bottleneck back to
    // channels (reference Layer constructor validation).
    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![StackConfig {
            input_size: 1,
            condition_size: 1,
            head_size: 1,
            channels: 2,
            bottleneck: 1,
            dilations: vec![1],
            kernel_sizes: vec![2],
            activation: ActivationConfig::from_name("Tanh").unwrap(),
            gating_modes: vec![GatingMode::None],
            secondary_activations: vec![None],
            groups_input: 1,
            groups_input_mixin: 1,
            layer1x1_groups: 1,
            head1x1: Head1x1Params::inactive(2),
        }],
        head: vec![],
        head_size: 1,
        head_bias: false,
        has_layer1x1: false,
    };
    let weights = counted_weights(64);
    let mut reader = WeightReader::new(&weights);
    let err = WaveNetModel::from_config_and_weights(config, &mut reader)
        .err()
        .expect("bottleneck != channels without layer1x1 must fail");
    assert!(err.contains("bottleneck"), "{err}");
}
