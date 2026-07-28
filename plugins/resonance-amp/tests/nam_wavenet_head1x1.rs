//! Tests for the optional head1x1 skip-path convolution (A2). When active,
//! a layer's contribution to the stack's skip/head accumulator is
//! `head1x1(activated z)` — bottleneck -> `head1x1.out_channels`, grouped,
//! always biased — instead of the activated z directly, and the skip
//! accumulator / head_rechannel input widen to `out_channels` (reference
//! `Layer` / `_head_output_size` in NAM/wavenet/detail.h + model.cpp).
//! Covers: the weight consumption position relative to layer1x1
//! (reference `Layer::set_weights_`: conv, input_mixin, layer1x1, head1x1),
//! grouped head1x1 inference, inactive bit-identity with the A1 path,
//! weight-count pins, the wavenet_a2_max fixture, and construction-time
//! validation errors.

use resonance_amp::nam::activations::{ActivationConfig, ActivationKind};
use resonance_amp::nam::parse::{
    load_model_from_file, parse_wavenet_config, StackConfig, WaveNetConfig, WeightReader,
};
use resonance_amp::nam::wavenet::params::{GatingMode, Head1x1Params, LayerFilms};
use resonance_amp::nam::wavenet::WaveNetModel;
use resonance_amp::nam::{fast_tanh, NamInference};

fn write_temp_nam(name: &str, body: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "resonance_amp_nam_head1x1_{}_{name}.nam",
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

/// Deterministic small filler weights (kept small to avoid saturation).
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

fn ungated_stack(
    channels: usize,
    bottleneck: usize,
    dilations: Vec<usize>,
    kernel_size: usize,
    head1x1: Head1x1Params,
) -> StackConfig {
    let n = dilations.len();
    StackConfig {
        input_size: 1,
        condition_size: 1,
        head_size: 1,
        head_kernel_size: 1,
        head_dilation: 1,
        head_bias: false,
        channels,
        bottleneck,
        dilations,
        kernel_sizes: vec![kernel_size; n],
        activations: vec![ActivationConfig::simple(ActivationKind::Tanh)],
        gating_modes: vec![GatingMode::None; n],
        secondary_activations: vec![None; n],
        groups_input: 1,
        groups_input_mixin: 1,
        layer1x1_active: true,
        layer1x1_groups: 1,
        head1x1,
        films: LayerFilms::default(),
    }
}

// -- Weight-order pin ----------------------------------------------------------

/// Two layers with BOTH layer1x1 and head1x1 active, every tensor holding
/// distinct values, hand-computed end to end. This pins the reference
/// consumption position of the head1x1 weights (`Layer::set_weights_`:
/// conv, input_mixin, layer1x1, then head1x1): if the engine read the
/// head1x1 tensors before the layer1x1 (or skipped a bias), layer 1's
/// residual and skip would swap/shift weights, layer 2 would consume
/// misaligned values, and the bit-exact expectation below would fail.
#[test]
fn head1x1_weight_order_matches_reference_position() {
    let r = [0.3f32, -0.4]; // rechannel 2x1
    let c1 = [0.5f32, 0.25, -0.3, 0.4]; // layer 1 conv 2x2 (kernel 1)
    let d1 = [0.05f32, -0.02]; // layer 1 conv bias
    let m1 = [0.6f32, -0.5]; // layer 1 mixin 2x1
    let l1 = [0.7f32, -0.2, 0.35, 0.45]; // layer 1 layer1x1 2x2
    let p1 = [0.01f32, -0.01]; // layer 1 layer1x1 bias
    let hw1 = [0.55f32, -0.15, 0.2, 0.65]; // layer 1 head1x1 2x2
    let hb1 = [0.02f32, -0.03]; // layer 1 head1x1 bias
    let c2 = [-0.35f32, 0.45, 0.15, -0.25]; // layer 2 conv 2x2
    let d2 = [0.02f32, 0.04]; // layer 2 conv bias
    let m2 = [-0.25f32, 0.55]; // layer 2 mixin 2x1
    let l2 = [0.3f32, 0.6, -0.4, 0.1]; // layer 2 layer1x1 (consumed, unused)
    let p2 = [0.0f32, 0.02]; // layer 2 layer1x1 bias
    let hw2 = [-0.45f32, 0.3, 0.5, -0.1]; // layer 2 head1x1 2x2
    let hb2 = [0.01f32, 0.04]; // layer 2 head1x1 bias
    let h = [0.9f32, -0.7]; // head_rechannel 1x2, no bias
    let s = 2.0f32; // head_scale

    let mut weights = Vec::new();
    for part in [
        &r[..],
        &c1,
        &d1,
        &m1,
        &l1,
        &p1,
        &hw1,
        &hb1,
        &c2,
        &d2,
        &m2,
        &l2,
        &p2,
        &hw2,
        &hb2,
        &h,
    ] {
        weights.extend_from_slice(part);
    }
    weights.push(s);
    assert_eq!(weights.len(), 45);

    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![ungated_stack(
            2,
            2,
            vec![1, 1],
            1,
            Head1x1Params {
                active: true,
                out_channels: 2,
                groups: 1,
            },
        )],
        head: vec![],
        head_size: 1,
        has_layer1x1: true,
        condition_dsp: None,
        fast_activations: true,
    };
    let mut reader = WeightReader::new(&weights);
    let mut model = WaveNetModel::from_config_and_weights(config, &mut reader)
        .expect("head1x1 model must construct");
    assert_eq!(reader.remaining(), 0, "all weights must be consumed");

    for &x in &[0.25f32, -0.5, 0.75] {
        let a = [r[0] * x, r[1] * x]; // rechannel; the condition is raw x
        // Layer 1.
        let z1 = [
            ((c1[0] * a[0] + c1[1] * a[1]) + d1[0]) + m1[0] * x,
            ((c1[2] * a[0] + c1[3] * a[1]) + d1[1]) + m1[1] * x,
        ];
        let t1 = [fast_tanh(z1[0]), fast_tanh(z1[1])];
        // Skip contribution: head1x1(t1) + bias (NOT t1 itself).
        let mut skip = [
            (hw1[0] * t1[0] + hw1[1] * t1[1]) + hb1[0],
            (hw1[2] * t1[0] + hw1[3] * t1[1]) + hb1[1],
        ];
        // Residual: layer1x1(t1) + bias, on top of the layer input.
        let a2 = [
            a[0] + ((l1[0] * t1[0] + l1[1] * t1[1]) + p1[0]),
            a[1] + ((l1[2] * t1[0] + l1[3] * t1[1]) + p1[1]),
        ];
        // Layer 2 (the raw input conditions every layer).
        let z2 = [
            ((c2[0] * a2[0] + c2[1] * a2[1]) + d2[0]) + m2[0] * x,
            ((c2[2] * a2[0] + c2[3] * a2[1]) + d2[1]) + m2[1] * x,
        ];
        let t2 = [fast_tanh(z2[0]), fast_tanh(z2[1])];
        skip[0] += (hw2[0] * t2[0] + hw2[1] * t2[1]) + hb2[0];
        skip[1] += (hw2[2] * t2[0] + hw2[3] * t2[1]) + hb2[1];
        // Head rechannel + scale (no extra skip activation).
        let expected = (h[0] * skip[0] + h[1] * skip[1]) * s;
        let out = model.process_sample(x);
        assert_eq!(
            out.to_bits(),
            expected.to_bits(),
            "head1x1 weight order must match the reference (got {out}, expected {expected})"
        );
    }
}

// -- Grouped head1x1 -----------------------------------------------------------

/// Grouped head1x1 widening the skip path: bottleneck 2 -> out_channels 4
/// with groups 2, so each per-group block is [2x1] (group 0: skip rows 0-1
/// from z[0]; group 1: rows 2-3 from z[1]) and the head_rechannel consumes a
/// 4-wide accumulator — the skip width follows head1x1.out_channels, not
/// bottleneck.
#[test]
fn grouped_head1x1_is_bit_identical_to_hand_computed_reference() {
    let r = [0.3f32, -0.4]; // rechannel 2x1
    let c = [0.5f32, 0.25, -0.3, 0.4]; // conv 2x2 (kernel 1, dense)
    let d = [0.05f32, -0.02]; // conv bias
    let m = [0.6f32, -0.5]; // mixin 2x1
    let hw = [0.55f32, -0.15, 0.2, 0.65]; // head1x1 g=2: two [2x1] blocks
    let hb = [0.02f32, -0.03, 0.01, 0.04]; // head1x1 bias [4]
    let h = [0.9f32, -0.7, 0.8, -0.6]; // head_rechannel 1x4, no bias
    let s = 2.0f32; // head_scale

    let mut weights = Vec::new();
    for part in [&r[..], &c, &d, &m, &hw, &hb, &h] {
        weights.extend_from_slice(part);
    }
    weights.push(s);
    assert_eq!(weights.len(), 23);

    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![ungated_stack(
            2,
            2,
            vec![1],
            1,
            Head1x1Params {
                active: true,
                out_channels: 4,
                groups: 2,
            },
        )],
        head: vec![],
        head_size: 1,
        has_layer1x1: false,
        condition_dsp: None,
        fast_activations: true,
    };
    let mut reader = WeightReader::new(&weights);
    let mut model = WaveNetModel::from_config_and_weights(config, &mut reader)
        .expect("grouped head1x1 model must construct");
    assert_eq!(reader.remaining(), 0);

    for &x in &[0.25f32, -0.5, 0.75] {
        let a = [r[0] * x, r[1] * x];
        let z = [
            ((c[0] * a[0] + c[1] * a[1]) + d[0]) + m[0] * x,
            ((c[2] * a[0] + c[3] * a[1]) + d[1]) + m[1] * x,
        ];
        let t = [fast_tanh(z[0]), fast_tanh(z[1])];
        // Grouped head1x1: group 0 maps t[0] to skip rows 0-1, group 1 maps
        // t[1] to rows 2-3. The accumulator feeds the head rechannel
        // directly (no extra activation).
        let skip = [
            hw[0] * t[0] + hb[0],
            hw[1] * t[0] + hb[1],
            hw[2] * t[1] + hb[2],
            hw[3] * t[1] + hb[3],
        ];
        let expected = (h[0] * skip[0] + h[1] * skip[1] + h[2] * skip[2] + h[3] * skip[3]) * s;
        let out = model.process_sample(x);
        assert_eq!(
            out.to_bits(),
            expected.to_bits(),
            "grouped head1x1 output must be bit-identical (got {out}, expected {expected})"
        );
    }
}

// -- Inactive bit-identity -----------------------------------------------------

/// An explicit inactive head1x1 object must construct the exact same model
/// as omitting the field entirely (the A1 default): same weight
/// consumption, bit-identical output stream. Its out_channels/groups values
/// are ignored while inactive (the reference only uses them when
/// constructing the conv).
#[test]
fn inactive_head1x1_is_bit_identical_to_a1_default() {
    let base = r#""input_size": 1, "condition_size": 1, "head_size": 1,
        "channels": 3, "bottleneck": 2,
        "dilations": [1, 2], "kernel_size": 2,
        "activation": "Tanh", "gating_mode": "gated",
        "head_bias": true"#;
    let with_field = format!(
        r#"{{"layers": [{{ {base},
            "head1x1": {{"active": false, "out_channels": 7, "groups": 5}} }}],
            "head": null, "head_scale": 1.0}}"#
    );
    let without_field =
        format!(r#"{{"layers": [{{ {base} }}], "head": null, "head_scale": 1.0}}"#);

    // channels=3, bottleneck=2, gated (mid 4), 2 layers, kernel 2:
    // rechannel 3 + 2*(conv 4*3*2=24 + bias 4 + mixin 4 + l1x1 6+3)
    // + head_rechannel 1*2+1=3 + head_scale 1.
    let count = 3 + 2 * (24 + 4 + 4 + 9) + 3 + 1;
    let weights = counted_weights(count);

    let mut explicit = load_nam("inactive", &new_format_json(&with_field, &weights))
        .expect("inactive head1x1 must load");
    let mut default = load_nam("absent", &new_format_json(&without_field, &weights))
        .expect("A1 default must load");

    for i in 0..32 {
        let x = ((i as f32) * 0.37).sin() * 0.8;
        let a = explicit.process_sample(x);
        let b = default.process_sample(x);
        assert!(a.is_finite());
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "inactive head1x1 must degenerate to the A1 direct-skip path exactly"
        );
    }
}

// -- Weight-count pin ----------------------------------------------------------

/// The wavenet_a2_max outer array's shape (channels 4, bottleneck 4, kernel
/// 4, dilations [1,2], grouped layer1x1 and head1x1) with the not-yet-
/// consumed surface stripped (FiLM, condition_dsp): the exact reference
/// count for the head1x1-extended layout loads, and a short vector
/// underflows — proving head1x1 consumes out*bottleneck/groups + out
/// weights per layer at the right place.
const HEAD1X1_CONFIG: &str = r#"{
    "layers": [{
        "input_size": 1, "condition_size": 1, "head_size": 1,
        "channels": 4, "bottleneck": 4,
        "dilations": [1, 2], "kernel_size": 4,
        "activation": "Tanh", "gating_mode": "none",
        "layer1x1": {"active": true, "groups": 2},
        "head1x1": {"active": true, "out_channels": 4, "groups": 2},
        "head_bias": true
    }],
    "head": null,
    "head_scale": 0.02
}"#;

/// rechannel 4*1 = 4
/// per layer: conv 4*4*4 = 64 + bias 4 + mixin 4
///            + layer1x1 4*4/2 = 8 + bias 4
///            + head1x1  4*4/2 = 8 + bias 4      -> 96, 2 layers = 192
/// head_rechannel 1*4 + 1(bias) = 5
/// head_scale = 1
/// total = 202
const HEAD1X1_TOTAL: usize = 202;

#[test]
fn loader_head1x1_weight_count_matches_reference() {
    let weights = counted_weights(HEAD1X1_TOTAL);
    let mut model = load_nam("count_exact", &new_format_json(HEAD1X1_CONFIG, &weights))
        .expect("exact head1x1 weight count must load");
    assert!(model.process_sample(0.1).is_finite());

    // Two weights short: construction must underflow before the trailing
    // (optional) head_scale read.
    let err = load_nam(
        "count_short",
        &new_format_json(HEAD1X1_CONFIG, &weights[..HEAD1X1_TOTAL - 2]),
    )
    .err()
    .expect("short head1x1 weight vector must fail");
    assert!(err.contains("Weight underflow"), "{err}");
}

// -- wavenet_a2_max fixture smoke ----------------------------------------------

/// The real A2 fixture's outer layer array has an active grouped head1x1.
/// The engine parse must surface it, and construction must consume the
/// fixture's real weight stream exactly (since todo #1109 the FiLM tensors
/// load too, so the full 818-weight outer model is consumed end to end;
/// per-site FiLM counts are pinned in nam_wavenet_film.rs).
#[test]
fn a2_max_fixture_head1x1_reaches_construction() {
    let path = format!(
        "{}/tests/fixtures/a2/wavenet_a2_max.nam",
        env!("CARGO_MANIFEST_DIR")
    );
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let config = parse_wavenet_config(file["config"].clone()).expect("fixture config must parse");

    assert_eq!(config.stacks.len(), 1);
    assert_eq!(
        config.stacks[0].head1x1,
        Head1x1Params {
            active: true,
            out_channels: 4,
            groups: 2,
        },
        "fixture head1x1 params must reach the engine config"
    );

    let weights: Vec<f32> = file["weights"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_f64().unwrap() as f32)
        .collect();
    assert_eq!(weights.len(), 818, "fixture outer weight stream");
    let mut reader = WeightReader::new(&weights);
    WaveNetModel::from_config_and_weights(config, &mut reader)
        .expect("fixture construction must succeed");
    assert_eq!(
        reader.remaining(),
        0,
        "the fixture's full weight stream must be consumed exactly"
    );
}

// -- Construction-time validation ----------------------------------------------

fn head1x1_json(head1x1: &str) -> String {
    format!(
        r#"{{"layers": [{{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 4, "bottleneck": 2,
            "dilations": [1], "kernel_size": 2,
            "activation": "Tanh", "gating_mode": "none",
            "head_bias": true, "head1x1": {head1x1} }}],
            "head": null, "head_scale": 1.0}}"#
    )
}

#[test]
fn head1x1_in_channels_not_divisible_by_groups_is_rejected() {
    // head1x1 input is the bottleneck (2); groups 3 does not divide it.
    let err = load_nam(
        "in_indiv",
        &new_format_json(
            &head1x1_json(r#"{"active": true, "out_channels": 6, "groups": 3}"#),
            &counted_weights(64),
        ),
    )
    .err()
    .expect("bottleneck % groups != 0 must fail");
    assert!(
        err.contains("head1x1 in_channels (2) must be divisible by groups (3)"),
        "{err}"
    );
}

#[test]
fn head1x1_out_channels_not_divisible_by_groups_is_rejected() {
    let err = load_nam(
        "out_indiv",
        &new_format_json(
            &head1x1_json(r#"{"active": true, "out_channels": 3, "groups": 2}"#),
            &counted_weights(64),
        ),
    )
    .err()
    .expect("out_channels % groups != 0 must fail");
    assert!(
        err.contains("head1x1 out_channels (3) must be divisible by groups (2)"),
        "{err}"
    );
}

#[test]
fn head1x1_zero_groups_is_rejected() {
    let err = load_nam(
        "zero_groups",
        &new_format_json(
            &head1x1_json(r#"{"active": true, "out_channels": 2, "groups": 0}"#),
            &counted_weights(64),
        ),
    )
    .err()
    .expect("groups == 0 must fail");
    assert!(err.contains("head1x1 groups must be >= 1"), "{err}");
}

#[test]
fn head1x1_object_missing_fields_is_rejected() {
    // All three fields are required when the object is present (the
    // reference JSON access throws on absence).
    let err = load_nam(
        "missing_fields",
        &new_format_json(
            &head1x1_json(r#"{"active": true}"#),
            &counted_weights(64),
        ),
    )
    .err()
    .expect("head1x1 object without out_channels/groups must fail");
    assert!(err.contains("Invalid WaveNet config"), "{err}");
}
