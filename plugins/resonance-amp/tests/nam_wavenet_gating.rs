//! Tests for the three A2 WaveNet gating modes (ba todo #1106):
//! `none` / `gated` / `blended` with config-driven primary + secondary
//! activations, per NAM/gating_activations.h of the NeuralAmpModelerCore
//! reference. Hand-computed tiny models pin the exact semantics; A1
//! bit-identity of the default gated path is asserted against the legacy
//! `gated: true` config.

use resonance_amp::nam::activations::{
    exact_sigmoid, hardswish, leaky_relu, ActivationConfig, ActivationKind,
};
use resonance_amp::nam::parse::{
    load_model_from_file, parse_full_wavenet_config, StackConfig, WaveNetConfig, WeightReader,
};
use resonance_amp::nam::wavenet::params::{GatingMode, Head1x1Params, LayerFilms};
use resonance_amp::nam::wavenet::WaveNetModel;
use resonance_amp::nam::NamInference;

fn write_temp_nam(name: &str, body: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "resonance_amp_nam_gating_{}_{name}.nam",
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

fn assert_close(out: f32, expected: f32, what: &str) {
    let tol = expected.abs().max(1.0) * 1e-6;
    assert!(
        (out - expected).abs() <= tol,
        "{what}: got {out}, expected {expected}"
    );
}

// -- Hand-computed tiny models ------------------------------------------------
//
// Single layer array, channels = bottleneck = 1, condition_size 1, one layer
// with kernel 1 / dilation 1 (no memory), head_bias false, no head MLP.
// A `gating_mode` key is an A2 marker, so these files run under REFERENCE
// semantics (ba todo #1113): the input rechannel is ALWAYS consumed and
// applied — even 1-to-1, it is a learned conv (reference LayerArray ctor)
// — the condition is the raw input x, there is NO extra activation between
// the skip accumulator and the head rechannel, and `Tanh`/`Sigmoid`
// resolve to the exact functions. The path per sample x is:
//   a        = rechannel * x
//   z_top    = conv_w[0]*a + conv_b[0] + mixin[0]*x
//   z_bottom = conv_w[1]*a + conv_b[1] + mixin[1]*x   (gated/blended only)
//   z        = <gating>(z_top, z_bottom)
//   out      = head_rechannel * z * head_scale
// (the layer1x1 residual only feeds the unused post-layer activation).
//
// Gated/blended weight order (mid = 2): rechannel[1], conv[2],
// conv bias[2], mixin[2], layer1x1 w[1] + b[1], head_rechannel[1],
// head_scale[1] -> 11 weights.
const TINY_WEIGHTS: [f32; 11] = [
    0.8, // rechannel (1x1, learned — deliberately != 1)
    0.7, -0.45, // conv w (top, bottom)
    0.05, 0.3, // conv bias (top, bottom)
    0.4, -0.15, // mixin (top, bottom)
    0.6, 0.02, // layer1x1 w, b
    0.9, // head_rechannel
    1.5, // head_scale
];

fn tiny_config(gating_mode: &str, activation: &str, secondary: Option<&str>) -> String {
    let secondary_field = match secondary {
        Some(s) => format!(r#""secondary_activation": {s},"#),
        None => String::new(),
    };
    format!(
        r#"{{
            "layers": [{{
                "input_size": 1, "condition_size": 1, "head_size": 1,
                "channels": 1,
                "dilations": [1], "kernel_size": 1,
                "activation": {activation},
                "gating_mode": "{gating_mode}",
                {secondary_field}
                "head_bias": false
            }}],
            "head": null,
            "head_scale": 1.5
        }}"#
    )
}

/// Pre-activation halves of the tiny model for input `x` (the rechanneled
/// input feeds the conv; the mixin reads the raw-input condition).
fn tiny_pre_activation(x: f32) -> (f32, f32) {
    let [r, w0, w1, b0, b1, m0, m1, ..] = TINY_WEIGHTS;
    let a = r * x;
    (w0 * a + b0 + m0 * x, w1 * a + b1 + m1 * x)
}

const TINY_INPUTS: [f32; 5] = [0.0, 0.25, -0.5, 0.8, -1.2];

/// gated with a configurable (non-default) secondary activation:
/// z = tanh(z_top) * hardswish(z_bottom).
#[test]
fn gated_honors_configured_secondary_activation() {
    let config = tiny_config("gated", r#""Tanh""#, Some(r#""Hardswish""#));
    let mut model =
        load_nam("gated_hardswish", &new_format_json(&config, &TINY_WEIGHTS)).unwrap();
    let [.., h, s] = TINY_WEIGHTS;

    for &x in &TINY_INPUTS {
        let (zt, zb) = tiny_pre_activation(x);
        let z = zt.tanh() * hardswish(zb);
        let expected = h * z * s;
        assert_close(model.process_sample(x), expected, "gated+Hardswish");
    }
}

/// blended per the reference BlendingActivation, with LeakyReLU primary and
/// Sigmoid blend: alpha = sigmoid(z_bottom) (exact, per reference
/// semantics), and
/// z = alpha * leaky_relu(z_top) + (1 - alpha) * z_top (pre-activation).
#[test]
fn blended_weighs_activated_against_pre_activation() {
    let config = tiny_config(
        "blended",
        r#"{"type": "LeakyReLU", "negative_slope": 0.2}"#,
        Some(r#""Sigmoid""#),
    );
    let mut model =
        load_nam("blended_leaky", &new_format_json(&config, &TINY_WEIGHTS)).unwrap();
    let [.., h, s] = TINY_WEIGHTS;

    for &x in &TINY_INPUTS {
        let (zt, zb) = tiny_pre_activation(x);
        let alpha = exact_sigmoid(zb);
        let z = alpha * leaky_relu(zt, 0.2) + (1.0 - alpha) * zt;
        let expected = h * z * s;
        assert_close(model.process_sample(x), expected, "blended+LeakyReLU");
    }
}

/// blended with a non-sigmoid blend activation (Hardswish, as in the
/// wavenet_a2_max fixture's condition_dsp): alpha comes straight from the
/// configured secondary.
#[test]
fn blended_honors_configured_blend_activation() {
    let config = tiny_config("blended", r#""Tanh""#, Some(r#""Hardswish""#));
    let mut model =
        load_nam("blended_hardswish", &new_format_json(&config, &TINY_WEIGHTS)).unwrap();
    let [.., h, s] = TINY_WEIGHTS;

    for &x in &TINY_INPUTS {
        let (zt, zb) = tiny_pre_activation(x);
        let alpha = hardswish(zb);
        let z = alpha * zt.tanh() + (1.0 - alpha) * zt;
        let expected = h * z * s;
        assert_close(model.process_sample(x), expected, "blended+Hardswish");
    }
}

// -- Default-secondary equivalence & the legacy/reference semantic gate -------

/// `gating_mode: "gated"` with no secondary must equal an explicit
/// `"Sigmoid"` secondary bit-for-bit (the reference backward-compat
/// default; both are A2-marked, so both use the exact sigmoid).
/// The legacy boolean `gated: true` carries NO A2 marker and keeps the A1
/// ACTIVATION FLAVOR (fast_tanh * fast_sigmoid) — the gate's only
/// remaining effect since #1116 (structure is shared) — so it must NOT
/// match the exact-flavor pair.
#[test]
fn a1_gated_default_and_explicit_sigmoid_secondary_are_bit_identical() {
    let base = r#""input_size": 1, "condition_size": 1, "head_size": 1,
        "channels": 2,
        "dilations": [1, 2], "kernel_size": 2,
        "activation": "Tanh",
        "head_bias": true"#;
    let variants = [
        format!(r#"{{"layers": [{{ {base}, "gating_mode": "gated" }}], "head": null, "head_scale": 1.0}}"#),
        format!(
            r#"{{"layers": [{{ {base}, "gating_mode": "gated", "secondary_activation": "Sigmoid" }}], "head": null, "head_scale": 1.0}}"#
        ),
        format!(r#"{{"layers": [{{ {base}, "gated": true }}], "head": null, "head_scale": 1.0}}"#),
    ];

    // channels=2, gated, 2 layers, kernel 2: rechannel 2
    // + 2*(conv 4*2*2=16 + bias 4 + mixin 4 + l1x1 4+2) + hr 1*2+1 + hs 1.
    let count = 2 + 2 * (16 + 4 + 4 + 6) + (2 + 1) + 1;
    let weights = counted_weights(count);

    let mut models: Vec<_> = variants
        .iter()
        .enumerate()
        .map(|(i, cfg)| {
            load_nam(&format!("a1_variant_{i}"), &new_format_json(cfg, &weights))
                .expect("A1 gated variant must load")
        })
        .collect();

    let mut legacy_differs = false;
    for i in 0..32 {
        let x = ((i as f32) * 0.41).sin() * 0.7;
        let outs: Vec<f32> = models.iter_mut().map(|m| m.process_sample(x)).collect();
        assert!(outs[0].is_finite());
        assert!(outs[2].is_finite());
        // Reference pair: the default secondary IS the (exact) Sigmoid.
        assert_eq!(
            outs[0].to_bits(),
            outs[1].to_bits(),
            "explicit Sigmoid secondary must equal the default secondary"
        );
        legacy_differs |= outs[0].to_bits() != outs[2].to_bits();
    }
    assert!(
        legacy_differs,
        "`gated: true` must keep the fast A1 activation flavor, distinct from the exact flavor"
    );
}

/// A single `gating_mode: "none"` string must be bit-identical to the
/// per-layer `["none", "none"]` array (both are A2 markers, both use the
/// exact activations). Omitting gating entirely is the A1 surface and
/// keeps the fast Tanh flavor — the gate's only remaining effect since
/// #1116 — so it must NOT match.
#[test]
fn gating_mode_none_is_bit_identical_to_per_layer_none_array() {
    let base = r#""input_size": 1, "condition_size": 1, "head_size": 1,
        "channels": 2,
        "dilations": [1, 2], "kernel_size": 2,
        "activation": "Tanh",
        "head_bias": true"#;
    let with_mode =
        format!(r#"{{"layers": [{{ {base}, "gating_mode": "none" }}], "head": null, "head_scale": 1.0}}"#);
    let with_array = format!(
        r#"{{"layers": [{{ {base}, "gating_mode": ["none", "none"] }}], "head": null, "head_scale": 1.0}}"#
    );
    let without = format!(r#"{{"layers": [{{ {base} }}], "head": null, "head_scale": 1.0}}"#);

    // Non-gated: rechannel 2 + 2*(conv 2*2*2=8 + bias 2 + mixin 2 + l1x1 6)
    // + hr 3 + hs 1.
    let count = 2 + 2 * (8 + 2 + 2 + 6) + 3 + 1;
    let weights = counted_weights(count);

    let mut a = load_nam("none_explicit", &new_format_json(&with_mode, &weights)).unwrap();
    let mut b = load_nam("none_array", &new_format_json(&with_array, &weights)).unwrap();
    let mut fast_flavor = load_nam("none_implicit", &new_format_json(&without, &weights)).unwrap();
    let mut legacy_differs = false;
    for i in 0..16 {
        let x = ((i as f32) * 0.53).cos() * 0.6;
        let ra = a.process_sample(x);
        let rb = b.process_sample(x);
        let rl = fast_flavor.process_sample(x);
        assert!(ra.is_finite());
        assert!(rl.is_finite());
        assert_eq!(ra.to_bits(), rb.to_bits());
        legacy_differs |= ra.to_bits() != rl.to_bits();
    }
    assert!(
        legacy_differs,
        "an A1 file without gating fields must keep the fast Tanh flavor, distinct from the exact flavor"
    );
}

// -- Mixed per-layer gating modes ---------------------------------------------

/// Mixed per-layer modes in one layer array: the ungated layer's conv/mixin
/// stay bottleneck-wide while the blended layer's are doubled; the exact
/// reference weight count is consumed with nothing left over.
#[test]
fn mixed_per_layer_gating_consumes_reference_weight_count() {
    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![StackConfig {
            input_size: 1,
            condition_size: 1,
            head_size: 1,
            head_kernel_size: 1,
            head_dilation: 1,
            head_bias: true,
            channels: 2,
            bottleneck: 2,
            dilations: vec![1, 2],
            kernel_sizes: vec![2, 2],
            activations: vec![ActivationConfig::simple(ActivationKind::Tanh)],
            gating_modes: vec![GatingMode::None, GatingMode::Blended],
            secondary_activations: vec![
                None,
                Some(ActivationConfig::simple(ActivationKind::Sigmoid)),
            ],
            groups_input: 1,
            groups_input_mixin: 1,
            layer1x1_active: true,
            layer1x1_groups: 1,
            head1x1: Head1x1Params::inactive(2),
            films: LayerFilms::default(),
        }],
        head: vec![],
        head_size: 1,
        has_layer1x1: true,
        condition_dsp: None,
        fast_activations: true,
    };
    // rechannel 2 + layer0 (mid 2: conv 8 + bias 2 + mixin 2 + l1x1 6 = 18)
    // + layer1 (mid 4: conv 16 + bias 4 + mixin 4 + l1x1 6 = 30)
    // + head_rechannel 1*2+1=3 + head_scale 1 = 54.
    let count = 2 + 18 + 30 + 3 + 1;
    let weights = counted_weights(count);
    let mut reader = WeightReader::new(&weights);
    let mut model = WaveNetModel::from_config_and_weights(config, &mut reader)
        .expect("mixed-gating construction must succeed");
    assert_eq!(reader.remaining(), 0, "all weights must be consumed");
    for i in 0..8 {
        assert!(model.process_sample(((i as f32) * 0.3).sin()).is_finite());
    }
}

/// Same mixed-mode config end-to-end through the .nam loader (per-layer
/// `gating_mode` array + single shared secondary).
#[test]
fn loader_accepts_per_layer_gating_mode_array() {
    let config = r#"{
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 2,
            "dilations": [1, 2], "kernel_size": 2,
            "activation": "Tanh",
            "gating_mode": ["none", "blended"],
            "secondary_activation": "Sigmoid",
            "head_bias": true
        }],
        "head": null,
        "head_scale": 1.0
    }"#;
    let count = 2 + 18 + 30 + 3 + 1;
    let weights = counted_weights(count);
    let mut model = load_nam("mixed_modes", &new_format_json(config, &weights))
        .expect("per-layer gating_mode array must load");
    for i in 0..8 {
        assert!(model.process_sample(((i as f32) * 0.3).cos()).is_finite());
    }
}

// -- Fixture-driven construction smoke ----------------------------------------

/// The wavenet_a2_max fixture's condition_dsp sub-model mixes blended and
/// gated layers with object-form secondary activations. Its typed gating
/// vectors (todo #1104) must drive construction directly: build a stack from
/// them, consume the exact per-layer weight counts, and stream finite audio.
#[test]
fn fixture_typed_gating_vectors_drive_construction() {
    let path = format!(
        "{}/tests/fixtures/a2/wavenet_a2_max.nam",
        env!("CARGO_MANIFEST_DIR")
    );
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let outer = parse_full_wavenet_config(&file["config"]).unwrap();
    let inner_json = outer.condition_dsp.expect("condition_dsp present");
    let inner = parse_full_wavenet_config(&inner_json["config"]).unwrap();

    let l1 = &inner.layer_arrays[1];
    assert_eq!(
        l1.gating_modes,
        vec![GatingMode::Blended, GatingMode::Gated, GatingMode::Gated]
    );
    assert!(l1.secondary_activations.iter().all(Option::is_some));

    // Build an engine stack from the fixture's typed per-layer gating (the
    // remaining A2 surface — per-layer primary activations, FiLM, groups —
    // lands with its own todos, so a single primary activation is used).
    let stack = StackConfig {
        input_size: 1,
        condition_size: 1,
        head_size: 1,
        head_kernel_size: 1,
        head_dilation: 1,
        head_bias: true,
        channels: l1.channels,
        bottleneck: l1.bottleneck,
        dilations: l1.dilations.clone(),
        kernel_sizes: l1.kernel_sizes.clone(),
        activations: vec![ActivationConfig::simple(ActivationKind::Softsign)],
        gating_modes: l1.gating_modes.clone(),
        secondary_activations: l1.secondary_activations.clone(),
        groups_input: 1,
        groups_input_mixin: 1,
        layer1x1_active: true,
        layer1x1_groups: 1,
        head1x1: Head1x1Params::inactive(l1.channels),
        films: LayerFilms::default(),
    };

    // Reference weight count for this stack.
    let ch = stack.channels;
    let bn = stack.bottleneck;
    let mut count = ch; // rechannel from input_size 1
    for (i, mode) in stack.gating_modes.iter().enumerate() {
        let mid = if *mode == GatingMode::None { bn } else { 2 * bn };
        count += mid * ch * stack.kernel_sizes[i]; // conv
        count += mid; // conv bias
        count += mid; // input mixin (condition_size 1)
        count += ch * bn + ch; // layer1x1 + bias
    }
    count += bn + 1; // head_rechannel (head_size 1) + bias
    count += 1; // head_scale

    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![stack],
        head: vec![],
        head_size: 1,
        has_layer1x1: true,
        condition_dsp: None,
        fast_activations: true,
    };
    let weights = counted_weights(count);
    let mut reader = WeightReader::new(&weights);
    let mut model = WaveNetModel::from_config_and_weights(config, &mut reader)
        .expect("fixture-derived gating vectors must construct");
    assert_eq!(reader.remaining(), 0, "all weights must be consumed");
    for i in 0..32 {
        assert!(model.process_sample(((i as f32) * 0.17).sin() * 0.5).is_finite());
    }
}
