//! Tests for the config-driven NAM activation registry:
//! scalar formulas for all activation kinds, string/object config parsing,
//! A1 fast-tanh mapping, and bit-identical A1 WaveNet inference.

use serde_json::json;

use resonance_amp::nam::activations::{Activation, ActivationConfig, ActivationKind};
use resonance_amp::nam::parse::load_model_from_file;
use resonance_amp::nam::{fast_tanh, sigmoid as fast_sigmoid};

const SAMPLE_POINTS: [f32; 7] = [-6.0, -2.0, -0.5, 0.0, 0.5, 2.0, 6.0];

fn apply_one(act: &Activation, x: f32) -> f32 {
    let mut buf = [x];
    act.apply(&mut buf);
    buf[0]
}

/// `apply` and `scalar` must agree (except PReLU per-channel, tested apart).
fn assert_scalar_matches_apply(act: &Activation) {
    for &x in &SAMPLE_POINTS {
        assert_eq!(act.scalar(x), apply_one(act, x), "{act:?} at {x}");
    }
}

// -- Scalar formulas ---------------------------------------------------------

#[test]
fn identity_passes_values_through() {
    let act = Activation::Identity;
    for &x in &SAMPLE_POINTS {
        assert_eq!(act.scalar(x), x);
    }
    assert_scalar_matches_apply(&act);
}

#[test]
fn tanh_matches_std_tanh() {
    let act = Activation::Tanh;
    for &x in &SAMPLE_POINTS {
        assert_eq!(act.scalar(x), x.tanh());
    }
    assert_scalar_matches_apply(&act);
}

#[test]
fn fast_tanh_matches_shared_primitive() {
    let act = Activation::FastTanh;
    for &x in &SAMPLE_POINTS {
        assert_eq!(act.scalar(x), fast_tanh(x));
    }
    assert_scalar_matches_apply(&act);
}

#[test]
fn hard_tanh_clamps_to_unit_range() {
    let act = Activation::HardTanh;
    assert_eq!(act.scalar(-2.0), -1.0);
    assert_eq!(act.scalar(-0.3), -0.3);
    assert_eq!(act.scalar(0.0), 0.0);
    assert_eq!(act.scalar(0.7), 0.7);
    assert_eq!(act.scalar(3.5), 1.0);
    assert_scalar_matches_apply(&act);
}

#[test]
fn leaky_hard_tanh_piecewise_formula() {
    let act = Activation::LeakyHardTanh {
        min_val: -1.0,
        max_val: 2.0,
        min_slope: 0.1,
        max_slope: 0.2,
    };
    // Below min: (x - min) * min_slope + min
    assert_eq!(act.scalar(-3.0), (-3.0f32 - -1.0) * 0.1 + -1.0);
    // Inside: identity
    assert_eq!(act.scalar(0.5), 0.5);
    assert_eq!(act.scalar(-1.0), -1.0);
    assert_eq!(act.scalar(2.0), 2.0);
    // Above max: (x - max) * max_slope + max
    assert_eq!(act.scalar(4.0), (4.0f32 - 2.0) * 0.2 + 2.0);
    assert_scalar_matches_apply(&act);
}

#[test]
fn relu_zeroes_negatives() {
    let act = Activation::Relu;
    assert_eq!(act.scalar(-2.0), 0.0);
    assert_eq!(act.scalar(0.0), 0.0);
    assert_eq!(act.scalar(1.5), 1.5);
    assert_scalar_matches_apply(&act);
}

#[test]
fn leaky_relu_scales_negatives() {
    let act = Activation::LeakyRelu {
        negative_slope: 0.2,
    };
    assert_eq!(act.scalar(-2.0), 0.2 * -2.0);
    assert_eq!(act.scalar(0.0), 0.0);
    assert_eq!(act.scalar(3.0), 3.0);
    assert_scalar_matches_apply(&act);
}

#[test]
fn prelu_cycles_per_channel_slopes() {
    let act = Activation::PRelu {
        negative_slopes: vec![0.1, 0.5],
    };
    // Positive values pass through regardless of channel.
    let mut buf = [1.0, 2.0, -1.0, -1.0];
    act.apply(&mut buf);
    // channel = pos % 2: slopes 0.1, 0.5, 0.1, 0.5
    assert_eq!(buf, [1.0, 2.0, -0.1, -0.5]);
    // scalar() uses the first slope.
    assert_eq!(act.scalar(-1.0), -0.1);
}

#[test]
fn sigmoid_is_exact_logistic() {
    let act = Activation::Sigmoid;
    for &x in &SAMPLE_POINTS {
        assert_eq!(act.scalar(x), 1.0 / (1.0 + (-x).exp()));
    }
    assert!((act.scalar(0.0) - 0.5).abs() < 1e-7);
    assert_scalar_matches_apply(&act);
}

#[test]
fn fast_sigmoid_matches_shared_primitive() {
    let act = Activation::FastSigmoid;
    for &x in &SAMPLE_POINTS {
        assert_eq!(act.scalar(x), fast_sigmoid(x));
        // The reference's exact form (NAM/activations.h fast_sigmoid).
        assert_eq!(act.scalar(x), 0.5 * (fast_tanh(x * 0.5) + 1.0));
    }
    assert_scalar_matches_apply(&act);
}

#[test]
fn silu_is_x_times_sigmoid() {
    let act = Activation::Silu;
    for &x in &SAMPLE_POINTS {
        assert_eq!(act.scalar(x), x * (1.0 / (1.0 + (-x).exp())));
    }
    assert_scalar_matches_apply(&act);
}

#[test]
fn hardswish_piecewise_formula() {
    let act = Activation::Hardswish;
    assert_eq!(act.scalar(-4.0), 0.0); // clamp at 0
    assert_eq!(act.scalar(0.0), 0.0);
    assert_eq!(act.scalar(1.0), 1.0 * 4.0 * (1.0 / 6.0));
    assert_eq!(act.scalar(-1.0), -2.0 * (1.0 / 6.0));
    assert_eq!(act.scalar(4.0), 4.0); // clamp at 6: x * 6/6
    assert_scalar_matches_apply(&act);
}

#[test]
fn softsign_formula() {
    let act = Activation::Softsign;
    for &x in &SAMPLE_POINTS {
        assert_eq!(act.scalar(x), x / (1.0 + x.abs()));
    }
    assert_scalar_matches_apply(&act);
}

// -- Config parsing: strings -------------------------------------------------

#[test]
fn all_activation_names_parse() {
    let cases = [
        ("Identity", ActivationKind::Identity),
        ("Tanh", ActivationKind::Tanh),
        ("Fasttanh", ActivationKind::FastTanh),
        ("Hardtanh", ActivationKind::HardTanh),
        ("LeakyHardtanh", ActivationKind::LeakyHardTanh),
        ("LeakyHardTanh", ActivationKind::LeakyHardTanh),
        ("ReLU", ActivationKind::Relu),
        ("LeakyReLU", ActivationKind::LeakyRelu),
        ("PReLU", ActivationKind::PRelu),
        ("Sigmoid", ActivationKind::Sigmoid),
        ("SiLU", ActivationKind::Silu),
        ("Hardswish", ActivationKind::Hardswish),
        ("Softsign", ActivationKind::Softsign),
    ];
    for (name, kind) in cases {
        let config = ActivationConfig::from_name(name).expect(name);
        assert_eq!(config.kind, kind, "{name}");
        // String form via from_json is equivalent.
        assert_eq!(ActivationConfig::from_json(&json!(name)).unwrap(), config);
    }
}

#[test]
fn unknown_activation_name_errors() {
    let err = ActivationConfig::from_name("Swishy").unwrap_err();
    assert!(err.contains("Unknown activation type"), "{err}");
    assert!(ActivationConfig::from_json(&json!("nope")).is_err());
}

#[test]
fn non_string_non_object_config_errors() {
    assert!(ActivationConfig::from_json(&json!(3)).is_err());
    assert!(ActivationConfig::from_json(&json!(["Tanh"])).is_err());
    assert!(ActivationConfig::from_json(&json!(null)).is_err());
}

// -- Config parsing: objects -------------------------------------------------

#[test]
fn object_config_requires_type_field() {
    let err = ActivationConfig::from_json(&json!({"negative_slope": 0.1})).unwrap_err();
    assert!(err.contains("type"), "{err}");
}

#[test]
fn leaky_relu_object_parses_slope_and_default() {
    let cfg = ActivationConfig::from_json(&json!({"type": "LeakyReLU", "negative_slope": 0.2}))
        .unwrap();
    assert_eq!(cfg.kind, ActivationKind::LeakyRelu);
    assert_eq!(cfg.negative_slope, Some(0.2));

    let cfg = ActivationConfig::from_json(&json!({"type": "LeakyReLU"})).unwrap();
    assert_eq!(cfg.negative_slope, Some(0.01));

    assert_eq!(
        Activation::from_config(&cfg, true),
        Activation::LeakyRelu {
            negative_slope: 0.01
        }
    );
}

#[test]
fn prelu_object_parses_single_and_per_channel_slopes() {
    let cfg =
        ActivationConfig::from_json(&json!({"type": "PReLU", "negative_slope": 0.3})).unwrap();
    assert_eq!(cfg.negative_slope, Some(0.3));
    assert_eq!(
        Activation::from_config(&cfg, true),
        Activation::PRelu {
            negative_slopes: vec![0.3]
        }
    );

    let cfg = ActivationConfig::from_json(
        &json!({"type": "PReLU", "negative_slopes": [0.1, 0.2, 0.3]}),
    )
    .unwrap();
    assert_eq!(cfg.negative_slopes, Some(vec![0.1, 0.2, 0.3]));
    assert_eq!(
        Activation::from_config(&cfg, true),
        Activation::PRelu {
            negative_slopes: vec![0.1, 0.2, 0.3]
        }
    );

    // Bare PReLU defaults to the reference's 0.01 single slope.
    let cfg = ActivationConfig::from_json(&json!({"type": "PReLU"})).unwrap();
    assert_eq!(
        Activation::from_config(&cfg, true),
        Activation::PRelu {
            negative_slopes: vec![0.01]
        }
    );

    assert!(
        ActivationConfig::from_json(&json!({"type": "PReLU", "negative_slopes": []})).is_err()
    );
}

#[test]
fn leaky_hardtanh_object_parses_params_and_defaults() {
    let cfg = ActivationConfig::from_json(&json!({
        "type": "LeakyHardtanh",
        "min_val": -2.0,
        "max_val": 3.0,
        "min_slope": 0.05,
        "max_slope": 0.15
    }))
    .unwrap();
    assert_eq!(
        Activation::from_config(&cfg, true),
        Activation::LeakyHardTanh {
            min_val: -2.0,
            max_val: 3.0,
            min_slope: 0.05,
            max_slope: 0.15
        }
    );

    let cfg = ActivationConfig::from_json(&json!({"type": "LeakyHardtanh"})).unwrap();
    assert_eq!(
        Activation::from_config(&cfg, true),
        Activation::LeakyHardTanh {
            min_val: -1.0,
            max_val: 1.0,
            min_slope: 0.01,
            max_slope: 0.01
        }
    );
}

// -- A1 resolution -----------------------------------------------------------

#[test]
fn a1_tanh_resolves_to_fast_tanh() {
    let cfg = ActivationConfig::from_name("Tanh").unwrap();
    // Model construction uses fast_tanh mode: "Tanh" -> fast tanh, exactly
    // the previously hardcoded A1 path.
    assert_eq!(Activation::from_config(&cfg, true), Activation::FastTanh);
    // Without fast-tanh mode, "Tanh" is the exact tanh.
    assert_eq!(Activation::from_config(&cfg, false), Activation::Tanh);
    // An explicit "Fasttanh" is always the fast path.
    let cfg = ActivationConfig::from_name("Fasttanh").unwrap();
    assert_eq!(Activation::from_config(&cfg, false), Activation::FastTanh);
}

// -- A1 WaveNet bit-identity through the registry ----------------------------

fn write_temp_nam(name: &str, body: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "resonance_amp_nam_activations_{}_{name}.nam",
        std::process::id()
    ));
    std::fs::write(&path, body).unwrap();
    path
}

/// Old-format gated WaveNet, 1 stack / 1 layer / 1 channel, kernel size 2.
/// Weight order: rechannel[1] (the reference consumes the 1-to-1 input
/// rechannel unconditionally; 1.0 keeps the hand math below unchanged),
/// w_conv[4] (raw layout [(out*ch+in)*ks+tap]), b_conv[2],
/// input_mixin[2], head_rechannel[1], head_scale[1].
const GATED_WEIGHTS: [f32; 11] = [1.0, 0.1, 0.2, 0.3, 0.4, 0.05, -0.05, 0.5, 0.6, 0.9, 2.0];

fn gated_wavenet_json() -> String {
    let weights: Vec<String> = GATED_WEIGHTS.iter().map(|w| w.to_string()).collect();
    format!(
        r#"{{
            "version": "0.5.2",
            "architecture": "WaveNet",
            "sample_rate": 48000,
            "config": {{
                "input_size": 1, "condition_size": 1, "head_size": 1,
                "channels": 1, "layers": [1], "head": [],
                "activation": "Tanh", "gated": true, "head_bias": false
            }},
            "weights": [{}]
        }}"#,
        weights.join(",")
    )
}

/// Replicate the A1-flavor gated WaveNet computation with the shared
/// fast_tanh / fast sigmoid primitives, in the exact operation order of
/// the engine (reference structure since #1116: raw-input condition, no
/// skip pre-activation; the 1.0 rechannel is a no-op numerically).
fn expected_gated_output(x: f32, prev: f32) -> f32 {
    let [_rc, w00, w01, w10, w11, b0, b1, m0, m1, hr, scale] = GATED_WEIGHTS;
    // conv_out[c] = ((w_tap0*prev) + (w_tap1*x)) + b[c] + (mixin[c]*x)
    let c0 = ((w00 * prev) + (w01 * x) + b0) + (m0 * x);
    let c1 = ((w10 * prev) + (w11 * x) + b1) + (m1 * x);
    // Gated activation: fast_tanh(z) * fast_sigmoid(g)
    let z = fast_tanh(c0) * fast_sigmoid(c1);
    // Head rechannel (no bias, no extra activation), head scale.
    (hr * z) * scale
}

#[test]
fn a1_gated_wavenet_output_is_bit_identical_to_fast_tanh_path() {
    let path = write_temp_nam("gated", &gated_wavenet_json());
    let loaded = load_model_from_file(path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);
    let mut model = loaded.expect("gated WaveNet should load").model;

    let inputs = [0.25f32, -0.5, 0.75];
    let mut prev = 0.0f32;
    for &x in &inputs {
        let out = model.process_sample(x);
        let expected = expected_gated_output(x, prev);
        assert_eq!(
            out.to_bits(),
            expected.to_bits(),
            "gated output must be bit-identical (got {out}, expected {expected})"
        );
        prev = x; // ring buffer stores the raw layer input
    }

    // reset() restores the initial state: the sequence repeats exactly.
    model.reset();
    assert_eq!(
        model.process_sample(inputs[0]).to_bits(),
        expected_gated_output(inputs[0], 0.0).to_bits()
    );
}

#[test]
fn a1_non_gated_wavenet_output_is_bit_identical_to_fast_tanh_path() {
    // Non-gated variant: rechannel[1] (1.0, see GATED_WEIGHTS), w_conv[2],
    // b_conv[1], mixin[1], head_rechannel[1], head_scale[1].
    let weights = [1.0f32, 0.1, 0.2, 0.05, 0.5, 0.9, 2.0];
    let body = format!(
        r#"{{
            "architecture": "WaveNet",
            "sample_rate": 48000,
            "config": {{
                "input_size": 1, "condition_size": 1, "head_size": 1,
                "channels": 1, "layers": [1], "head": [],
                "activation": "Tanh", "gated": false, "head_bias": false
            }},
            "weights": [{}]
        }}"#,
        weights
            .iter()
            .map(|w| w.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let path = write_temp_nam("nongated", &body);
    let loaded = load_model_from_file(path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);
    let mut model = loaded.expect("non-gated WaveNet should load").model;

    let [_rc, w0, w1, b, m, hr, scale] = weights;
    let mut prev = 0.0f32;
    for &x in &[0.25f32, -0.5, 0.75] {
        let out = model.process_sample(x);
        let z = fast_tanh(((w0 * prev) + (w1 * x) + b) + (m * x));
        // No skip pre-activation (reference structure since #1116).
        let expected = (hr * z) * scale;
        assert_eq!(out.to_bits(), expected.to_bits());
        prev = x;
    }
}

#[test]
fn unknown_wavenet_activation_is_rejected() {
    let body = r#"{
        "architecture": "WaveNet",
        "config": {
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 1, "layers": [1], "head": [],
            "activation": "Blorp", "gated": false, "head_bias": false
        },
        "weights": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    }"#;
    let path = write_temp_nam("unknown_act", body);
    let result = load_model_from_file(path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);
    let err = result.err().expect("unknown activation should fail");
    assert!(err.contains("Unknown activation type"), "{err}");
}
