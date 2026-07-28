//! Tests for the A2 windowed head rechannel (`head_kernel_size` /
//! `head_dilation`): each stack's head rechannel is a causal dilated
//! convolution over the accumulated skip signal (reference
//! `_head_rechannel` = `Conv1D(skip_ch, head_size, head_kernel_size,
//! head_bias, head_dilation, 1)` in NAM/wavenet/model.cpp, processed with
//! no extra activation in `LayerArray::ProcessInner`). Covers: hand-computed
//! kernel taps + dilation offsets, the Conv1D `[out][in][tap]` weight
//! de-interleave, weight-count pins, the kernel-1 reduction to the legacy
//! (A1) head path bit-for-bit, ring reset, construction-time validation,
//! and the A2.nam fixture's kernel-16 head geometry.

use resonance_amp::nam::activations::{ActivationConfig, ActivationKind};
use resonance_amp::nam::parse::{
    load_model_from_file, parse_full_wavenet_config, parse_wavenet_config, StackConfig,
    WaveNetConfig, WeightReader,
};
use resonance_amp::nam::wavenet::params::{GatingMode, Head1x1Params, HeadParams};
use resonance_amp::nam::wavenet::WaveNetModel;
use resonance_amp::nam::NamInference;

fn write_temp_nam(name: &str, body: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "resonance_amp_nam_windowed_head_{}_{name}.nam",
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

/// Single-stack config with a windowed head rechannel. ReLU activation so
/// hand computations stay exact for positive signals.
fn windowed_stack(
    channels: usize,
    head_kernel_size: usize,
    head_dilation: usize,
    head_bias: bool,
) -> StackConfig {
    StackConfig {
        input_size: 1,
        condition_size: 1,
        head_size: 1,
        head_kernel_size,
        head_dilation,
        head_bias,
        channels,
        bottleneck: channels,
        dilations: vec![1],
        kernel_sizes: vec![1],
        activation: ActivationConfig::simple(ActivationKind::Relu),
        gating_modes: vec![GatingMode::None],
        secondary_activations: vec![None],
        groups_input: 1,
        groups_input_mixin: 1,
        layer1x1_groups: 1,
        head1x1: Head1x1Params::inactive(channels),
    }
}

// -- Hand-computed taps + dilation offsets -----------------------------------

/// channels=1, one kernel-1 layer (ReLU), windowed head kernel 3 with
/// dilation 2 and bias. An impulse isolates every tap: the skip stream is
/// S[t] = relu((a + m) * x[t] + b), and the head output must be
/// `w0*S[t-4] + w1*S[t-2] + w2*S[t] + hb` — tap 0 is the OLDEST frame,
/// tap ks-1 the current one, spaced `head_dilation` apart (reference
/// `Conv1D::Process` offsets `dilation * (k + 1 - kernel_size)`).
#[test]
fn windowed_head_taps_and_dilation_offsets_are_bit_exact() {
    let a = 0.5f32; // conv weight
    let b = 0.0f32; // conv bias
    let m = 0.25f32; // input mixin
    let (w0, w1, w2) = (0.5f32, 0.25f32, 2.0f32); // head taps, oldest first
    let hb = 0.125f32; // head bias

    // Weight order: conv, conv bias, mixin, head taps ([out][in][tap] with
    // out = in = 1 is just the tap sequence), head bias, head_scale.
    let weights = vec![a, b, m, w0, w1, w2, hb, 1.0];
    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![windowed_stack(1, 3, 2, true)],
        head: vec![],
        head_size: 1,
        has_layer1x1: false,
    };
    let mut reader = WeightReader::new(&weights);
    let mut model = WaveNetModel::from_config_and_weights(config, &mut reader)
        .expect("windowed-head model must construct");
    assert_eq!(reader.remaining(), 0, "all weights must be consumed");

    let s0 = (a + m) * 1.0 + b; // skip frame for the impulse sample (ReLU no-op)
    let expected = [
        w2 * s0 + hb, // t=0: current-frame tap
        hb,           // t=1: gap (dilation 2)
        w1 * s0 + hb, // t=2: middle tap, 1*dilation back
        hb,           // t=3: gap
        w0 * s0 + hb, // t=4: oldest tap, (ks-1)*dilation back
        hb,           // t=5: impulse left the window
        hb,           // t=6
    ];
    for (t, &want) in expected.iter().enumerate() {
        let x = if t == 0 { 1.0 } else { 0.0 };
        let got = model.process_sample(x);
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "t={t}: got {got}, expected {want}"
        );
    }
}

/// Reset must clear the windowed head's history ring: replaying the impulse
/// after reset() reproduces the exact same output sequence.
#[test]
fn reset_clears_windowed_head_history() {
    let weights = vec![0.5, 0.0, 0.25, 0.5, 0.25, 2.0, 0.125, 1.0];
    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![windowed_stack(1, 3, 2, true)],
        head: vec![],
        head_size: 1,
        has_layer1x1: false,
    };
    let mut reader = WeightReader::new(&weights);
    let mut model = WaveNetModel::from_config_and_weights(config, &mut reader).unwrap();

    let run = |model: &mut WaveNetModel| -> Vec<u32> {
        (0..6)
            .map(|t| model.process_sample(if t == 0 { 1.0 } else { 0.0 }).to_bits())
            .collect()
    };
    let first = run(&mut model);
    model.reset();
    let second = run(&mut model);
    assert_eq!(first, second, "reset must clear windowed head history");
}

// -- Weight de-interleave ([out][in][tap]) -----------------------------------

/// skip width 2, head kernel 2: the flat head weights follow the reference
/// `Conv1D::set_weights_` order `[out][in][tap]`, so
/// raw = [w(i0,t0), w(i0,t1), w(i1,t0), w(i1,t1)] where t0 multiplies the
/// PREVIOUS frame and t1 the current one.
#[test]
fn windowed_head_weight_order_deinterleaves_conv1d_layout() {
    let r = [1.0f32, 0.5]; // rechannel 2x1
    let c = [0.5f32, 0.0, 0.0, 0.25]; // conv 2x2 (kernel 1), diagonal-ish
    let d = [0.0f32, 0.0]; // conv bias
    let m = [0.0f32, 0.0]; // mixin (zeroed: keeps hand math short)
    let h = [0.5f32, 2.0, 0.25, 4.0]; // head raw: [in0: t0, t1][in1: t0, t1]
    let s = 1.0f32; // head_scale

    let mut weights = Vec::new();
    for part in [&r[..], &c, &d, &m, &h] {
        weights.extend_from_slice(part);
    }
    weights.push(s);

    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![windowed_stack(2, 2, 1, false)],
        head: vec![],
        head_size: 1,
        has_layer1x1: false,
    };
    let mut reader = WeightReader::new(&weights);
    let mut model = WaveNetModel::from_config_and_weights(config, &mut reader)
        .expect("2-channel windowed-head model must construct");
    assert_eq!(reader.remaining(), 0);

    // Skip frames (ReLU exact for positive values):
    //   S(x) = [c00 * r0 * x, c11 * r1 * x]
    let skip = |x: f32| [c[0] * (r[0] * x), c[3] * (r[1] * x)];

    let x0 = 1.0f32;
    let x1 = 0.5f32;
    let s0 = skip(x0);
    let s1 = skip(x1);

    // t=0: previous frame is the zeroed ring -> only current-frame taps
    // (h[1], h[3]); t=1: previous taps (h[0], h[2]) see s0.
    let want0 = h[1] * s0[0] + h[3] * s0[1];
    let want1 = (h[0] * s0[0] + h[2] * s0[1]) + (h[1] * s1[0] + h[3] * s1[1]);

    let got0 = model.process_sample(x0);
    let got1 = model.process_sample(x1);
    assert_eq!(got0.to_bits(), want0.to_bits(), "t=0: got {got0}, expected {want0}");
    assert_eq!(got1.to_bits(), want1.to_bits(), "t=1: got {got1}, expected {want1}");
}

// -- Weight-count pins -------------------------------------------------------

/// Windowed head weight consumption: kernel taps multiply the 1x1 count by
/// `head_kernel_size`; bias unchanged. Exact count constructs with nothing
/// left over, one fewer fails.
#[test]
fn windowed_head_weight_count_is_pinned() {
    // channels=2 (skip width 2), head_size=1, kernel 3:
    //   rechannel 2 + conv 4 + bias 2 + mixin 2
    //   + head 1*2*3 = 6 + head bias 1 + head_scale 1 = 18
    let count = 2 + 4 + 2 + 2 + 6 + 1 + 1;
    let make_config = || WaveNetConfig {
        input_size: 1,
        stacks: vec![windowed_stack(2, 3, 4, true)],
        head: vec![],
        head_size: 1,
        has_layer1x1: false,
    };

    let weights = counted_weights(count);
    let mut reader = WeightReader::new(&weights);
    let mut model = WaveNetModel::from_config_and_weights(make_config(), &mut reader)
        .expect("exact weight count must construct");
    assert_eq!(reader.remaining(), 0, "all weights must be consumed");
    for i in 0..16 {
        assert!(model.process_sample(((i as f32) * 0.3).sin()).is_finite());
    }

    // The trailing head_scale is optional in the engine (defaults to 1.0),
    // so shorten by two: the head-bias read then underflows.
    let short = counted_weights(count - 2);
    let mut reader = WeightReader::new(&short);
    assert!(
        WaveNetModel::from_config_and_weights(make_config(), &mut reader).is_err(),
        "missing weight must fail construction"
    );
}

/// head_kernel_size 0 is rejected at construction (mirrors the reference
/// `head_kernel_size must be >= 1` validation).
#[test]
fn zero_head_kernel_size_is_rejected() {
    let config = WaveNetConfig {
        input_size: 1,
        stacks: vec![windowed_stack(1, 0, 1, true)],
        head: vec![],
        head_size: 1,
        has_layer1x1: false,
    };
    let weights = counted_weights(16);
    let mut reader = WeightReader::new(&weights);
    let err = WaveNetModel::from_config_and_weights(config, &mut reader)
        .err()
        .expect("kernel 0 must be rejected");
    assert!(err.contains("head_kernel_size"), "unexpected error: {err}");
}

// -- Kernel-1 reduction to the legacy (A1) head path -------------------------

const KERNEL1_WEIGHTS: [f32; 17] = [
    0.4, -0.3, // rechannel 2x1
    0.5, -0.25, 0.1, 0.35, // conv 2x2
    0.05, -0.05, // conv bias
    0.3, -0.2, // input mixin 2x1
    0.6, -0.1, 0.2, 0.45, // layer1x1 2x2
    0.02, -0.03, // layer1x1 bias
    // (head rechannel 1x2 + bias appended per-variant below is shared too)
    0.2, // head rechannel: first of 1x2
];

fn kernel1_json(head_field: &str) -> String {
    let ws: Vec<String> = KERNEL1_WEIGHTS
        .iter()
        .copied()
        .chain([-0.6f32, 0.15, 1.5]) // head rechannel tail, head bias, head_scale
        .map(|w| w.to_string())
        .collect();
    format!(
        r#"{{
            "architecture": "WaveNet",
            "sample_rate": 48000,
            "config": {{
                "layers": [{{
                    "input_size": 1, "condition_size": 1, {head_field},
                    "channels": 2, "dilations": [1], "kernel_size": 1,
                    "activation": "Tanh", "gated": false
                }}],
                "head": null,
                "head_scale": 1.5
            }},
            "weights": [{}]
        }}"#,
        ws.join(",")
    )
}

/// A nested A2 `head` object with kernel_size 1 must reduce to the legacy
/// flat `head_size`/`head_bias` path bit-for-bit: same weight layout, same
/// output (including the legacy skip pre-activation). `head_dilation` is
/// irrelevant at kernel 1 (the window is a single frame).
#[test]
fn nested_head_kernel1_is_bit_identical_to_legacy_flat() {
    // Legacy A1-format layer array (implicit kernel-1 head).
    let legacy = kernel1_json(r#""head_size": 1, "head_bias": true"#);
    // Same model through the A2 nested head object.
    let nested = kernel1_json(r#""head": {"out_channels": 1, "kernel_size": 1, "bias": true}"#);
    // And with a (meaningless at kernel 1) head_dilation.
    let nested_dilated = kernel1_json(
        r#""head": {"out_channels": 1, "kernel_size": 1, "head_dilation": 7, "bias": true}"#,
    );

    let mut legacy = load_nam("legacy", &legacy).expect("legacy flat head must load");
    let mut nested = load_nam("nested", &nested).expect("nested kernel-1 head must load");
    let mut nested_dilated =
        load_nam("nested_dilated", &nested_dilated).expect("dilated kernel-1 head must load");

    for i in 0..64 {
        let x = ((i as f32) * 0.37).sin() * 0.8;
        let a = legacy.process_sample(x);
        let b = nested.process_sample(x);
        let c = nested_dilated.process_sample(x);
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "sample {i}: nested kernel-1 head diverged from legacy ({b} vs {a})"
        );
        assert_eq!(
            a.to_bits(),
            c.to_bits(),
            "sample {i}: head_dilation must be inert at kernel 1 ({c} vs {a})"
        );
    }
}

/// The legacy MLP head path is untouched: an A1-style model with head MLP
/// layers still constructs with the historical weight layout (this guards
/// the windowed-head rework against consuming head weights differently).
#[test]
fn legacy_mlp_head_weight_layout_unchanged() {
    // Old flat format: channels=2, 1 stack x 2 layers (kernel 2), head [2].
    // rechannel none (input_size == channels? no: old format input_size 1).
    let body = r#"{
        "architecture": "WaveNet",
        "sample_rate": 48000,
        "config": {
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 2, "layers": [2], "head": [2],
            "activation": "Tanh", "gated": false, "head_bias": true
        },
        "weights": [WS]
    }"#;
    // Old format weight count: rechannel 2*1 + 2 layers (conv 2*2*2 + bias 2
    // + mixin 2*1) + head_rechannel 1*2 + hr bias 1 + head MLP: hidden 2x1+2
    // + final 1x2+1 + head_scale 1.
    let count = 2 + 2 * (8 + 2 + 2) + 2 + 1 + (2 + 2) + (2 + 1) + 1;
    let ws: Vec<String> = counted_weights(count).iter().map(|w| w.to_string()).collect();
    let mut model = load_nam("legacy_mlp", &body.replace("WS", &ws.join(",")))
        .expect("legacy MLP-head model must load with the historical layout");
    for i in 0..16 {
        assert!(model.process_sample(((i as f32) * 0.21).sin()).is_finite());
    }
}

// -- Parse surface -----------------------------------------------------------

/// Engine parse: nested head object populates the per-stack windowed-head
/// fields; legacy flat fields imply kernel 1; both absent is an error;
/// kernel 0 is an error.
#[test]
fn parse_wavenet_config_resolves_nested_and_legacy_head() {
    let nested: serde_json::Value = serde_json::from_str(
        r#"{
            "layers": [{
                "input_size": 1, "condition_size": 1, "channels": 2,
                "dilations": [1, 2], "kernel_size": 3, "activation": "Tanh",
                "gated": false,
                "head": {"out_channels": 4, "kernel_size": 16, "head_dilation": 2, "bias": false}
            }],
            "head": null, "head_scale": 0.02
        }"#,
    )
    .unwrap();
    let cfg = parse_wavenet_config(nested).expect("nested head must parse");
    assert_eq!(cfg.stacks[0].head_size, 4);
    assert_eq!(cfg.stacks[0].head_kernel_size, 16);
    assert_eq!(cfg.stacks[0].head_dilation, 2);
    assert!(!cfg.stacks[0].head_bias);
    assert_eq!(cfg.head_size, 4, "config head_size follows the resolved head");

    let legacy: serde_json::Value = serde_json::from_str(
        r#"{
            "layers": [{
                "input_size": 1, "condition_size": 1, "channels": 2,
                "head_size": 1, "head_bias": true,
                "dilations": [1], "kernel_size": 2, "activation": "Tanh",
                "gated": false
            }],
            "head": null, "head_scale": 0.02
        }"#,
    )
    .unwrap();
    let cfg = parse_wavenet_config(legacy).expect("legacy head must parse");
    assert_eq!(cfg.stacks[0].head_kernel_size, 1);
    assert_eq!(cfg.stacks[0].head_dilation, 1);
    assert!(cfg.stacks[0].head_bias);

    let neither: serde_json::Value = serde_json::from_str(
        r#"{
            "layers": [{
                "input_size": 1, "condition_size": 1, "channels": 2,
                "dilations": [1], "kernel_size": 2, "activation": "Tanh",
                "gated": false
            }],
            "head": null, "head_scale": 0.02
        }"#,
    )
    .unwrap();
    let err = parse_wavenet_config(neither).err().expect("head-less layer must fail");
    assert!(err.contains("head"), "unexpected error: {err}");

    let zero_kernel: serde_json::Value = serde_json::from_str(
        r#"{
            "layers": [{
                "input_size": 1, "condition_size": 1, "channels": 2,
                "dilations": [1], "kernel_size": 2, "activation": "Tanh",
                "gated": false,
                "head": {"out_channels": 1, "kernel_size": 0, "bias": true}
            }],
            "head": null, "head_scale": 0.02
        }"#,
    )
    .unwrap();
    let err = parse_wavenet_config(zero_kernel)
        .err()
        .expect("kernel 0 must fail");
    assert!(err.contains("kernel_size"), "unexpected error: {err}");
}

// -- A2.nam fixture ----------------------------------------------------------

/// The A2.nam fixture's WaveNet submodels use a windowed head
/// (`head: {out_channels: 1, kernel_size: 16, bias: true}`). The typed A2
/// parser reports it, the engine parse resolves it, and a model with the
/// submodel's exact head geometry constructs and runs. (The fixture's real
/// weights are slimmable-packed — unpacking is todo #1112 — and its
/// per-layer activation arrays are not yet wired into the engine config, so
/// construction uses the parsed geometry with synthetic weights and the
/// shared LeakyReLU activation.)
#[test]
fn a2_fixture_submodel_windowed_head_geometry_constructs() {
    let raw = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/a2/A2.nam"),
    )
    .expect("A2.nam fixture must be present");
    let file: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let sub0 = &file["config"]["submodels"][0]["model"]["config"];

    // Typed A2 parse sees the windowed head on the layer array.
    let typed = parse_full_wavenet_config(sub0).expect("typed parse of A2 submodel 0");
    let arr = &typed.layer_arrays[0];
    assert_eq!(arr.head_size, 1);
    assert_eq!(arr.head_kernel_size, 16);
    assert_eq!(arr.head_dilation, 1);
    assert!(arr.head_bias);
    assert!(
        typed.head.is_none(),
        "fixture has no post-stack head (\"head\": null)"
    );

    // Engine parse: collapse the (uniformly identical) per-layer activation
    // array to a single config — per-layer activations land with their own
    // todo — then resolve the nested head.
    let mut engine_json = sub0.clone();
    let acts = engine_json["layers"][0]["activation"].as_array().unwrap().clone();
    assert!(
        acts.iter().all(|a| *a == acts[0]),
        "fixture layer activations are uniform"
    );
    engine_json["layers"][0]["activation"] = acts[0].clone();
    let cfg = parse_wavenet_config(engine_json).expect("engine parse of A2 submodel 0");
    let stack = &cfg.stacks[0];
    assert_eq!(stack.head_kernel_size, 16);
    assert_eq!(stack.head_dilation, 1);
    assert!(stack.head_bias);
    assert_eq!(stack.channels, 3);
    assert_eq!(stack.bottleneck, 3);
    assert_eq!(stack.dilations.len(), 23);

    // Reference weight count for the plain (non-slimmable) layout of this
    // geometry, windowed head included.
    let ch = stack.channels;
    let bn = stack.bottleneck;
    let mut count = ch; // rechannel from input_size 1
    for ks in &stack.kernel_sizes {
        count += bn * ch * ks + bn; // conv + bias (ungated: mid == bn)
        count += bn * stack.condition_size; // input mixin
        count += ch * bn + ch; // layer1x1 + bias
    }
    count += stack.head_size * bn * stack.head_kernel_size; // head taps
    count += stack.head_size; // head bias
    count += 1; // head_scale

    // Small alternating weights: the fixture's LeakyReLU is unbounded, so
    // larger synthetic weights would blow up across its 23 residual layers.
    let weights: Vec<f32> = (0..count)
        .map(|i| ((i % 11) as f32 - 5.0) * 0.002)
        .collect();
    let mut reader = WeightReader::new(&weights);
    let mut model = WaveNetModel::from_config_and_weights(cfg, &mut reader)
        .expect("A2 submodel geometry must construct");
    assert_eq!(reader.remaining(), 0, "all weights must be consumed");
    for i in 0..64 {
        assert!(model.process_sample(((i as f32) * 0.13).sin() * 0.5).is_finite());
    }
}

/// The typed parser also models the (unused-by-fixtures) post-stack
/// windowed head; sanity-pin the variant so this todo's surface stays in
/// sync with params.rs.
#[test]
fn typed_post_stack_head_variant_still_parses() {
    let cfg: serde_json::Value = serde_json::from_str(
        r#"{
            "layers": [{
                "input_size": 1, "condition_size": 1, "channels": 2,
                "head_size": 4, "head_bias": true,
                "dilations": [1], "kernel_size": 2, "activation": "Tanh",
                "gated": false
            }],
            "head": {
                "channels": 8, "out_channels": 1, "kernel_sizes": [16, 1],
                "activation": "ReLU"
            },
            "head_scale": 1.0
        }"#,
    )
    .unwrap();
    let typed = parse_full_wavenet_config(&cfg).expect("windowed post-stack head parses");
    match typed.head {
        Some(HeadParams::Windowed {
            in_channels,
            channels,
            out_channels,
            ref kernel_sizes,
            ..
        }) => {
            assert_eq!(in_channels, 4);
            assert_eq!(channels, 8);
            assert_eq!(out_channels, 1);
            assert_eq!(kernel_sizes, &vec![16, 1]);
        }
        other => panic!("expected Windowed head, got {other:?}"),
    }
}
