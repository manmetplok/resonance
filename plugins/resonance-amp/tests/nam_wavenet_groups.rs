//! Tests for grouped convolutions in the WaveNet inference path (todo
//! groups_input / groups_input_mixin / layer1x1.groups): reference-matching
//! weight consumption for grouped variants, hand-computed grouped inference,
//! groups == 1 bit-identity with the dense path, and construction-time
//! divisibility validation (reference NAM/conv1d.cpp + NAM/dsp.cpp).

use resonance_amp::nam::activations::{ActivationConfig, ActivationKind};
use resonance_amp::nam::parse::{load_model_from_file, StackConfig, WaveNetConfig, WeightReader};
use resonance_amp::nam::wavenet::params::{GatingMode, Head1x1Params};
use resonance_amp::nam::wavenet::WaveNetModel;
use resonance_amp::nam::{fast_tanh, NamInference};

fn write_temp_nam(name: &str, body: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "resonance_amp_nam_groups_{}_{name}.nam",
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

#[allow(clippy::too_many_arguments)]
fn grouped_stack(
    channels: usize,
    bottleneck: usize,
    condition_size: usize,
    dilations: Vec<usize>,
    kernel_size: usize,
    gated: bool,
    groups_input: usize,
    groups_input_mixin: usize,
    layer1x1_groups: usize,
    head_bias: bool,
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
        condition_size,
        head_size: 1,
        head_kernel_size: 1,
        head_dilation: 1,
        head_bias,
        channels,
        bottleneck,
        dilations,
        kernel_sizes: vec![kernel_size; n],
        activation: ActivationConfig::simple(ActivationKind::Tanh),
        gating_modes,
        secondary_activations,
        groups_input,
        groups_input_mixin,
        layer1x1_groups,
        head1x1: Head1x1Params::inactive(channels),
    }
}

// -- Weight-count pins for grouped variants -----------------------------------

/// Non-gated, channels=4, bottleneck=4, condition=2, kernel 2, 2 layers,
/// groups_input=2, groups_input_mixin=2, layer1x1.groups=4. Reference
/// consumption (each grouped tensor shrinks to out*in*ks/g):
///   rechannel          4*1              =  4
///   per layer: conv    4*4*2/2 = 16, bias 4,
///              mixin   4*2/2   =  4,
///              layer1x1 4*4/4  =  4 + bias 4   -> 32 each, 2 layers = 64
///   head_rechannel     1*4 + 1(bias)    =  5
///   head_scale                          =  1
///   total                               = 74
const GROUPED_TOTAL: usize = 74;

#[test]
fn grouped_construction_consumes_reference_weight_count() {
    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![grouped_stack(4, 4, 2, vec![1, 2], 2, false, 2, 2, 4, true)],
        head: vec![],
        head_size: 1,
        has_layer1x1: true,
    };
    let weights = counted_weights(GROUPED_TOTAL);
    let mut reader = WeightReader::new(&weights);
    let mut model = WaveNetModel::from_config_and_weights(config, &mut reader)
        .expect("grouped construction must succeed");
    assert_eq!(reader.remaining(), 0, "all weights must be consumed");
    for i in 0..8 {
        assert!(model.process_sample(((i as f32) * 0.3).sin()).is_finite());
    }
}

/// Gated grouped variant: the conv/mixin output 2*bottleneck channels and
/// the group count divides that doubled width. channels=4, bottleneck=4
/// (mid 8), condition=2, 1 layer, kernel 2, groups 2/2/2:
///   rechannel      4*1        =  4
///   conv           8*4*2/2    = 32, bias 8
///   mixin          8*2/2      =  8
///   layer1x1       4*4/2 + 4  = 12
///   head_rechannel 1*4 + 1    =  5
///   head_scale                =  1
///   total                     = 70
#[test]
fn gated_grouped_construction_consumes_reference_weight_count() {
    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![grouped_stack(4, 4, 2, vec![1], 2, true, 2, 2, 2, true)],
        head: vec![],
        head_size: 1,
        has_layer1x1: true,
    };
    let weights = counted_weights(70);
    let mut reader = WeightReader::new(&weights);
    WaveNetModel::from_config_and_weights(config, &mut reader)
        .expect("gated grouped construction must succeed");
    assert_eq!(reader.remaining(), 0, "all weights must be consumed");
}

/// The same grouped shape end-to-end through the .nam loader: the JSON
/// groups fields (incl. the layer1x1 object) must reach construction, the
/// exact reference count loads, and a short vector underflows.
const GROUPED_CONFIG: &str = r#"{
    "layers": [{
        "input_size": 1, "condition_size": 2, "head_size": 1,
        "channels": 4, "bottleneck": 4,
        "dilations": [1, 2], "kernel_size": 2,
        "activation": "Tanh", "gating_mode": "none",
        "groups_input": 2, "groups_input_mixin": 2,
        "layer1x1": {"active": true, "groups": 4},
        "head_bias": true
    }],
    "head": null,
    "head_scale": 0.02
}"#;

#[test]
fn loader_grouped_weight_count_matches_reference() {
    let weights = counted_weights(GROUPED_TOTAL);
    let mut model = load_nam("grouped_exact", &new_format_json(GROUPED_CONFIG, &weights))
        .expect("exact grouped weight count must load");
    assert!(model.process_sample(0.1).is_finite());

    // Two weights short: construction must underflow before the trailing
    // (optional) head_scale read.
    let err = load_nam(
        "grouped_short",
        &new_format_json(GROUPED_CONFIG, &weights[..GROUPED_TOTAL - 2]),
    )
    .err()
    .expect("short grouped weight vector must fail");
    assert!(err.contains("Weight underflow"), "{err}");
}

// -- Hand-computed grouped inference ------------------------------------------

/// 4 channels, groups_input=2 (two 2x2 blocks), single layer with kernel 1
/// (no memory) and no layer1x1, so the whole computation is hand-checkable.
/// Weight order (reference Conv1D::set_weights_, [group][out][in][tap]):
/// group 0 covers channels {0,1}, group 1 covers {2,3}; each output block
/// sees only its own input block.
#[test]
fn grouped_conv_output_is_bit_identical_to_hand_computed_reference() {
    let r = [0.3f32, -0.4, 0.5, 0.2]; // rechannel 4x1
    // conv g=2, kernel 1: flat [g][i][j] = [w00, w01, w10, w11, w22, w23, w32, w33]
    let w = [0.5f32, 0.25, -0.3, 0.4, 0.6, -0.2, 0.15, 0.35];
    let b = [0.05f32, -0.02, 0.03, 0.01]; // conv bias
    let m = [0.6f32, -0.5, 0.4, -0.3]; // mixin 4x1 (condition = channel 0)
    let h = [0.9f32, -0.7, 0.8, -0.6]; // head_rechannel 1x4, no bias
    let s = 2.0f32; // head_scale

    let mut weights = Vec::new();
    weights.extend_from_slice(&r);
    weights.extend_from_slice(&w);
    weights.extend_from_slice(&b);
    weights.extend_from_slice(&m);
    weights.extend_from_slice(&h);
    weights.push(s);

    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![StackConfig {
            input_size: 1,
            condition_size: 1,
            head_size: 1,
            head_kernel_size: 1,
            head_dilation: 1,
            head_bias: false,
            channels: 4,
            bottleneck: 4,
            dilations: vec![1],
            kernel_sizes: vec![1],
            activation: ActivationConfig::simple(ActivationKind::Tanh),
            gating_modes: vec![GatingMode::None],
            secondary_activations: vec![None],
            groups_input: 2,
            groups_input_mixin: 1,
            layer1x1_groups: 1,
            head1x1: Head1x1Params::inactive(4),
        }],
        head: vec![],
        head_size: 1,
        has_layer1x1: false,
    };
    let mut reader = WeightReader::new(&weights);
    let mut model = WaveNetModel::from_config_and_weights(config, &mut reader)
        .expect("grouped conv model must construct");
    assert_eq!(reader.remaining(), 0);

    for &x in &[0.25f32, -0.5, 0.75] {
        let a = [r[0] * x, r[1] * x, r[2] * x, r[3] * x];
        // Group 0: outputs 0..2 from inputs 0..2; group 1: outputs 2..4
        // from inputs 2..4. Mixin adds m_c * condition (channel 0 of the
        // rechanneled input) after the conv bias — engine op order.
        let z = [
            ((w[0] * a[0] + w[1] * a[1]) + b[0]) + m[0] * a[0],
            ((w[2] * a[0] + w[3] * a[1]) + b[1]) + m[1] * a[0],
            ((w[4] * a[2] + w[5] * a[3]) + b[2]) + m[2] * a[0],
            ((w[6] * a[2] + w[7] * a[3]) + b[3]) + m[3] * a[0],
        ];
        // Layer activation, then the stack's skip pre-activation.
        let t: Vec<f32> = z.iter().map(|&v| fast_tanh(fast_tanh(v))).collect();
        let expected = (h[0] * t[0] + h[1] * t[1] + h[2] * t[2] + h[3] * t[3]) * s;
        let out = model.process_sample(x);
        assert_eq!(
            out.to_bits(),
            expected.to_bits(),
            "grouped conv output must be bit-identical (got {out}, expected {expected})"
        );
    }
}

/// Depthwise-grouped everything (2 channels, all three group counts = 2,
/// condition_size 2, kernel 1) across TWO layers, so the grouped layer1x1
/// demonstrably shapes the residual feeding layer 2 and the grouped mixin
/// projects the 2-wide condition per-channel.
#[test]
fn grouped_mixin_and_layer1x1_are_bit_identical_to_hand_computed_reference() {
    let r = [0.3f32, -0.4]; // rechannel 2x1 (dense)
    let c1 = [0.5f32, 0.25]; // layer 1 conv, g=2 (one weight per channel)
    let d1 = [0.05f32, -0.02]; // layer 1 conv bias
    let m1 = [0.6f32, -0.5]; // layer 1 mixin, g=2
    let l1 = [0.7f32, -0.2]; // layer 1 layer1x1, g=2
    let p1 = [0.01f32, -0.01]; // layer 1 layer1x1 bias
    let c2 = [-0.35f32, 0.45]; // layer 2 conv, g=2
    let d2 = [0.02f32, 0.04]; // layer 2 conv bias
    let m2 = [-0.25f32, 0.55]; // layer 2 mixin, g=2
    let l2 = [0.3f32, 0.6]; // layer 2 layer1x1, g=2 (consumed, residual unused)
    let p2 = [0.0f32, 0.02]; // layer 2 layer1x1 bias
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
        &c2,
        &d2,
        &m2,
        &l2,
        &p2,
        &h,
    ] {
        weights.extend_from_slice(part);
    }
    weights.push(s);
    assert_eq!(weights.len(), 25);

    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![grouped_stack(2, 2, 2, vec![1, 1], 1, false, 2, 2, 2, false)],
        head: vec![],
        head_size: 1,
        has_layer1x1: true,
    };
    let mut reader = WeightReader::new(&weights);
    let mut model = WaveNetModel::from_config_and_weights(config, &mut reader)
        .expect("grouped 2-layer model must construct");
    assert_eq!(reader.remaining(), 0);

    for &x in &[0.25f32, -0.5, 0.75] {
        let a = [r[0] * x, r[1] * x]; // rechannel; also the condition signal
        // Layer 1 (all convs depthwise-grouped: channel c sees channel c).
        let z1 = [
            ((c1[0] * a[0]) + d1[0]) + m1[0] * a[0],
            ((c1[1] * a[1]) + d1[1]) + m1[1] * a[1],
        ];
        let t1 = [fast_tanh(z1[0]), fast_tanh(z1[1])];
        // Grouped layer1x1 residual into layer 2's input.
        let a2 = [
            a[0] + (l1[0] * t1[0] + p1[0]),
            a[1] + (l1[1] * t1[1] + p1[1]),
        ];
        // Layer 2 (condition is still the stack input snapshot `a`).
        let z2 = [
            ((c2[0] * a2[0]) + d2[0]) + m2[0] * a[0],
            ((c2[1] * a2[1]) + d2[1]) + m2[1] * a[1],
        ];
        let t2 = [fast_tanh(z2[0]), fast_tanh(z2[1])];
        // Skip accumulator + skip pre-activation + head rechannel + scale.
        let sk = [fast_tanh(t1[0] + t2[0]), fast_tanh(t1[1] + t2[1])];
        let expected = (h[0] * sk[0] + h[1] * sk[1]) * s;
        let out = model.process_sample(x);
        assert_eq!(
            out.to_bits(),
            expected.to_bits(),
            "grouped mixin/layer1x1 output must be bit-identical (got {out}, expected {expected})"
        );
    }
}

// -- groups == 1 degeneracy ----------------------------------------------------

/// Explicit `groups_input`/`groups_input_mixin`/`layer1x1.groups` of 1 must
/// construct the exact same model as omitting the fields (the A1 default):
/// same weight consumption, bit-identical output stream — the dense fast
/// path stays untouched.
#[test]
fn explicit_unit_groups_are_bit_identical_to_a1_default() {
    let base = r#""input_size": 1, "condition_size": 1, "head_size": 1,
        "channels": 3, "bottleneck": 2,
        "dilations": [1, 2], "kernel_size": 2,
        "activation": "Tanh", "gating_mode": "gated",
        "head_bias": true"#;
    let with_fields = format!(
        r#"{{"layers": [{{ {base},
            "groups_input": 1, "groups_input_mixin": 1,
            "layer1x1": {{"active": true, "groups": 1}} }}],
            "head": null, "head_scale": 1.0}}"#
    );
    let without_fields =
        format!(r#"{{"layers": [{{ {base} }}], "head": null, "head_scale": 1.0}}"#);

    // channels=3, bottleneck=2, gated (mid 4), 2 layers, kernel 2:
    // rechannel 3 + 2*(conv 4*3*2=24 + bias 4 + mixin 4 + l1x1 6+3)
    // + head_rechannel 1*2+1=3 + head_scale 1.
    let count = 3 + 2 * (24 + 4 + 4 + 9) + 3 + 1;
    let weights = counted_weights(count);

    let mut explicit = load_nam("g1_explicit", &new_format_json(&with_fields, &weights))
        .expect("explicit unit groups must load");
    let mut default = load_nam("g1_default", &new_format_json(&without_fields, &weights))
        .expect("A1 default must load");

    for i in 0..32 {
        let x = ((i as f32) * 0.37).sin() * 0.8;
        let a = explicit.process_sample(x);
        let b = default.process_sample(x);
        assert!(a.is_finite());
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "groups == 1 must degenerate to the dense A1 path exactly"
        );
    }
}

// -- Construction-time divisibility validation --------------------------------

fn grouped_json(extra: &str) -> String {
    format!(
        r#"{{"layers": [{{
            "input_size": 1, "condition_size": 2, "head_size": 1,
            "channels": 4, "bottleneck": 2,
            "dilations": [1], "kernel_size": 2,
            "activation": "Tanh", "gating_mode": "none",
            "head_bias": true, {extra} }}],
            "head": null, "head_scale": 1.0}}"#
    )
}

#[test]
fn conv_in_channels_not_divisible_by_groups_is_rejected() {
    // channels=3 with groups_input=2 (reference Conv1D::set_size_ throws).
    let config = r#"{"layers": [{
        "input_size": 1, "condition_size": 1, "head_size": 1,
        "channels": 3, "dilations": [1], "kernel_size": 2,
        "activation": "Tanh", "gating_mode": "none",
        "head_bias": true, "groups_input": 2 }],
        "head": null, "head_scale": 1.0}"#;
    let err = load_nam("conv_in_indiv", &new_format_json(config, &counted_weights(64)))
        .err()
        .expect("in_channels % groups != 0 must fail");
    assert!(
        err.contains("conv in_channels (3) must be divisible by groups (2)"),
        "{err}"
    );
}

#[test]
fn conv_out_channels_not_divisible_by_groups_is_rejected() {
    // channels=4 divisible by 4, but the conv's output (bottleneck=2) isn't.
    let err = load_nam(
        "conv_out_indiv",
        &new_format_json(&grouped_json(r#""groups_input": 4"#), &counted_weights(64)),
    )
    .err()
    .expect("out_channels % groups != 0 must fail");
    assert!(
        err.contains("conv out_channels (2) must be divisible by groups (4)"),
        "{err}"
    );
}

#[test]
fn mixin_condition_size_not_divisible_by_groups_is_rejected() {
    // condition_size=2 with groups_input_mixin=3.
    let err = load_nam(
        "mixin_indiv",
        &new_format_json(
            &grouped_json(r#""groups_input_mixin": 3"#),
            &counted_weights(64),
        ),
    )
    .err()
    .expect("condition_size % groups != 0 must fail");
    assert!(
        err.contains("input_mixin in_channels (2) must be divisible by groups (3)"),
        "{err}"
    );
}

#[test]
fn layer1x1_channels_not_divisible_by_groups_is_rejected() {
    // layer1x1 maps bottleneck=2 -> channels=4; groups=3 divides neither.
    let err = load_nam(
        "l1x1_indiv",
        &new_format_json(
            &grouped_json(r#""layer1x1": {"active": true, "groups": 3}"#),
            &counted_weights(64),
        ),
    )
    .err()
    .expect("layer1x1 channels % groups != 0 must fail");
    assert!(
        err.contains("layer1x1 in_channels (2) must be divisible by groups (3)"),
        "{err}"
    );
}

#[test]
fn zero_groups_is_rejected() {
    let err = load_nam(
        "zero_groups",
        &new_format_json(&grouped_json(r#""groups_input": 0"#), &counted_weights(64)),
    )
    .err()
    .expect("groups == 0 must fail");
    assert!(err.contains("groups must be >= 1"), "{err}");
}
