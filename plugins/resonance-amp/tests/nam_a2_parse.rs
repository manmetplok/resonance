//! Parse tests for the full A2 WaveNet config surface (ba todo #1104).
//!
//! Loads the real A2 fixture files from `tests/fixtures/a2/` (MIT,
//! NeuralAmpModelerCore example_models — see the fixture README) and asserts
//! the typed values, plus handcrafted snippets for defaults and error cases.
//! Parse-only: these tests never construct models.

use serde_json::{json, Value};

use resonance_amp::nam::activations::{ActivationConfig, ActivationKind};
use resonance_amp::nam::parse::parse_full_wavenet_config;
use resonance_amp::nam::wavenet::params::{
    FilmParams, GatingMode, Head1x1Params, HeadParams, Layer1x1Params, WaveNetFullConfig,
};

fn fixture_json(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/a2/{name}", env!("CARGO_MANIFEST_DIR"));
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"));
    serde_json::from_str(&data).unwrap_or_else(|e| panic!("fixture {path} is not JSON: {e}"))
}

/// The `config` object of a WaveNet-architecture fixture file.
fn fixture_config(name: &str) -> Value {
    let file = fixture_json(name);
    assert_eq!(file["architecture"], "WaveNet", "{name} architecture");
    file["config"].clone()
}

fn film(active: bool, shift: bool, groups: usize) -> FilmParams {
    FilmParams {
        active,
        shift,
        groups,
    }
}

fn simple(kind: ActivationKind) -> ActivationConfig {
    ActivationConfig::simple(kind)
}

// -- Real fixture: wavenet_a2_max.nam (every A2 field exercised) --------------
//
// The outer config has one layer array conditioned on a full A2 condition_dsp
// sub-model; the nested sub-model's own WaveNet config carries the remaining
// surface (bottleneck, gated/blended, per-layer arrays, secondary activation).

#[test]
fn a2_max_top_level_surface() {
    let cfg = parse_full_wavenet_config(&fixture_config("wavenet_a2_max.nam")).unwrap();
    assert_eq!(cfg.layer_arrays.len(), 1);
    assert_eq!(cfg.head, None);
    assert_eq!(cfg.head_scale, Some(0.02));
    assert_eq!(cfg.in_channels, 1);

    // condition_dsp is kept as the raw nested model JSON for now.
    let dsp = cfg.condition_dsp.expect("condition_dsp present");
    assert_eq!(dsp["architecture"], "WaveNet");
    assert!(dsp["config"]["layers"].is_array());
}

#[test]
fn a2_max_outer_layer_groups_head1x1_films() {
    let cfg = parse_full_wavenet_config(&fixture_config("wavenet_a2_max.nam")).unwrap();
    let l0 = &cfg.layer_arrays[0];

    assert_eq!(l0.input_size, 1);
    // Conditioned on the condition_dsp sub-network's 8-channel output.
    assert_eq!(l0.condition_size, 8);
    assert_eq!(l0.channels, 4);
    assert_eq!(l0.bottleneck, 4);
    assert_eq!(l0.dilations, vec![1, 2]);
    // Single kernel_size 4 duplicated per layer.
    assert_eq!(l0.kernel_sizes, vec![4, 4]);

    // Legacy flat head fields: head_size + head_bias, implicit kernel 1.
    assert_eq!(l0.head_size, 1);
    assert_eq!(l0.head_kernel_size, 1);
    assert_eq!(l0.head_dilation, 1);
    assert!(l0.head_bias);

    // Single activation object duplicated per layer.
    assert_eq!(l0.activations, vec![simple(ActivationKind::Softsign); 2]);

    // gating_mode "none": the file's stray `"secondary_activation": ""` is
    // ignored, exactly like the reference parser.
    assert_eq!(l0.gating_modes, vec![GatingMode::None; 2]);
    assert_eq!(l0.secondary_activations, vec![None; 2]);

    // Grouped convolutions.
    assert_eq!(l0.groups_input, 1);
    assert_eq!(l0.groups_input_mixin, 4);
    assert_eq!(
        l0.layer1x1,
        Layer1x1Params {
            active: true,
            groups: 2
        }
    );
    assert_eq!(
        l0.head1x1,
        Head1x1Params {
            active: true,
            out_channels: 4,
            groups: 2
        }
    );

    // All 8 FiLM insertion points active with shift, per-point groups.
    assert_eq!(l0.conv_pre_film, film(true, true, 2));
    assert_eq!(l0.conv_post_film, film(true, true, 4));
    assert_eq!(l0.input_mixin_pre_film, film(true, true, 4));
    assert_eq!(l0.input_mixin_post_film, film(true, true, 2));
    assert_eq!(l0.activation_pre_film, film(true, true, 1));
    assert_eq!(l0.activation_post_film, film(true, true, 2));
    assert_eq!(l0.layer1x1_post_film, film(true, true, 8));
    assert_eq!(l0.head1x1_post_film, film(true, true, 4));

    assert_eq!(l0.slimmable, None);
}

/// The nested condition_dsp sub-model's own WaveNet config parses through the
/// same full-surface path: bottleneck != channels, single "gated" mode with a
/// named secondary activation, and all 8 FiLM points.
#[test]
fn a2_max_condition_dsp_inner_layer0_bottleneck_and_gating() {
    let outer = parse_full_wavenet_config(&fixture_config("wavenet_a2_max.nam")).unwrap();
    let inner_json = outer.condition_dsp.expect("condition_dsp present");
    let cfg = parse_full_wavenet_config(&inner_json["config"]).unwrap();
    assert_eq!(cfg.layer_arrays.len(), 2);
    assert_eq!(cfg.head, None);
    assert_eq!(cfg.head_scale, Some(0.02));
    let l0 = &cfg.layer_arrays[0];

    assert_eq!(l0.channels, 3);
    assert_eq!(l0.bottleneck, 6);
    assert_eq!(l0.kernel_sizes, vec![2, 2]);
    assert_eq!(l0.head_size, 4);
    assert!(!l0.head_bias);

    assert_eq!(l0.activations, vec![simple(ActivationKind::Silu); 2]);
    assert_eq!(l0.gating_modes, vec![GatingMode::Gated; 2]);
    assert_eq!(
        l0.secondary_activations,
        vec![Some(simple(ActivationKind::Hardswish)); 2]
    );

    assert_eq!(l0.groups_input, 3);
    assert_eq!(l0.groups_input_mixin, 1);
    assert_eq!(
        l0.layer1x1,
        Layer1x1Params {
            active: true,
            groups: 3
        }
    );
    assert_eq!(
        l0.head1x1,
        Head1x1Params {
            active: true,
            out_channels: 6,
            groups: 3
        }
    );
    for f in [
        l0.conv_pre_film,
        l0.conv_post_film,
        l0.input_mixin_pre_film,
        l0.input_mixin_post_film,
        l0.activation_pre_film,
        l0.activation_post_film,
        l0.layer1x1_post_film,
        l0.head1x1_post_film,
    ] {
        assert_eq!(f, film(true, true, 1));
    }
}

#[test]
fn a2_max_condition_dsp_inner_layer1_per_layer_arrays_and_blended_gating() {
    let outer = parse_full_wavenet_config(&fixture_config("wavenet_a2_max.nam")).unwrap();
    let inner_json = outer.condition_dsp.expect("condition_dsp present");
    let cfg = parse_full_wavenet_config(&inner_json["config"]).unwrap();
    let l1 = &cfg.layer_arrays[1];

    assert_eq!(l1.channels, 4);
    assert_eq!(l1.bottleneck, 2);
    assert_eq!(l1.dilations, vec![1, 3, 5]);
    assert_eq!(l1.kernel_sizes, vec![3, 3, 3]);
    assert_eq!(l1.head_size, 8);

    // Per-layer activation array: PReLU (per-channel slopes) x2 + Softsign.
    let prelu = |slopes: &[f32]| ActivationConfig {
        negative_slopes: Some(slopes.to_vec()),
        ..simple(ActivationKind::PRelu)
    };
    assert_eq!(
        l1.activations,
        vec![
            prelu(&[0.04, 0.05]),
            prelu(&[0.03, 0.01]),
            simple(ActivationKind::Softsign),
        ]
    );

    // Mixed per-layer gating modes including "blended".
    assert_eq!(
        l1.gating_modes,
        vec![GatingMode::Blended, GatingMode::Gated, GatingMode::Gated]
    );

    // Paired per-layer secondary activations.
    let leaky_hardtanh = ActivationConfig {
        min_val: Some(0.0),
        max_val: Some(0.9),
        min_slope: Some(0.0),
        max_slope: Some(0.02),
        ..simple(ActivationKind::LeakyHardTanh)
    };
    assert_eq!(
        l1.secondary_activations,
        vec![
            Some(leaky_hardtanh),
            Some(simple(ActivationKind::Relu)),
            Some(simple(ActivationKind::Sigmoid)),
        ]
    );

    // FiLM blocks active without shift on this array.
    assert_eq!(l1.conv_pre_film, film(true, false, 1));
    assert_eq!(l1.head1x1_post_film, film(true, false, 1));
    assert_eq!(
        l1.head1x1,
        Head1x1Params {
            active: true,
            out_channels: 4,
            groups: 2
        }
    );
}

// -- Real fixture: slimmable_wavenet.nam --------------------------------------

#[test]
fn slimmable_fixture_parses_descriptor_and_a1_defaults() {
    let cfg = parse_full_wavenet_config(&fixture_config("slimmable_wavenet.nam")).unwrap();
    assert_eq!(cfg.layer_arrays.len(), 1);
    let l = &cfg.layer_arrays[0];

    let slim = l.slimmable.as_ref().expect("slimmable descriptor");
    assert_eq!(slim.allowed_channels, vec![1, 2, 3]);

    // Everything else is a plain A1 config; A2 fields sit at their defaults.
    assert_eq!(l.channels, 3);
    assert_eq!(l.bottleneck, 3);
    assert_eq!(l.activations, vec![simple(ActivationKind::Relu); 10]);
    assert_eq!(l.gating_modes, vec![GatingMode::None; 10]);
    assert_eq!(l.secondary_activations, vec![None; 10]);
    assert_eq!(l.groups_input, 1);
    assert_eq!(l.groups_input_mixin, 1);
    assert_eq!(l.layer1x1, Layer1x1Params::default());
    assert_eq!(
        l.head1x1,
        Head1x1Params {
            active: false,
            out_channels: 3,
            groups: 1
        }
    );
    assert_eq!(l.conv_pre_film, FilmParams::default());
    assert_eq!(l.head_kernel_size, 1);
    assert_eq!(l.head_dilation, 1);
    assert_eq!(cfg.condition_dsp, None);
    assert_eq!(cfg.head, None);
    assert_eq!(cfg.head_scale, Some(0.02));
}

// -- Real fixture: wavenet_condition_dsp.nam ----------------------------------

#[test]
fn condition_dsp_fixture_keeps_raw_submodel_and_parses_outer_layers() {
    let cfg = parse_full_wavenet_config(&fixture_config("wavenet_condition_dsp.nam")).unwrap();

    let dsp = cfg.condition_dsp.expect("condition_dsp present");
    assert_eq!(dsp["architecture"], "WaveNet");
    assert_eq!(dsp["sample_rate"], 48000);

    // Outer layers condition on the sub-network's 3-channel output.
    assert_eq!(cfg.layer_arrays.len(), 2);
    assert_eq!(cfg.layer_arrays[0].condition_size, 3);
    assert_eq!(cfg.layer_arrays[1].condition_size, 3);
    assert_eq!(
        cfg.layer_arrays[0].activations,
        vec![simple(ActivationKind::Tanh); 2]
    );
    assert!(cfg.layer_arrays[1].head_bias);
}

// -- Real fixture: A2.nam (SlimmableContainer with WaveNet submodels) ---------

#[test]
fn a2_container_submodel_wavenet_configs_parse() {
    let file = fixture_json("A2.nam");
    assert_eq!(file["architecture"], "SlimmableContainer");
    let submodels = file["config"]["submodels"]
        .as_array()
        .expect("submodels array");
    assert_eq!(submodels.len(), 2);

    // A2-Lite and A2-Full submodels: same shape, different widths.
    for (sm, expected_channels) in submodels.iter().zip([3usize, 8]) {
        let model = &sm["model"];
        assert_eq!(model["architecture"], "WaveNet");
        let cfg = parse_full_wavenet_config(&model["config"]).unwrap();

        assert_eq!(cfg.layer_arrays.len(), 1);
        let l = &cfg.layer_arrays[0];
        assert_eq!(l.channels, expected_channels);
        assert_eq!(l.bottleneck, expected_channels);
        assert_eq!(l.dilations.len(), 23);
        assert_eq!(l.kernel_sizes.len(), 23);
        assert_eq!(l.kernel_sizes[0], 6);

        // Nested per-array head object: windowed rechannel (kernel 16).
        assert_eq!(l.head_size, 1);
        assert_eq!(l.head_kernel_size, 16);
        assert_eq!(l.head_dilation, 1);
        assert!(l.head_bias);

        // Per-layer activation array: 23x LeakyReLU(0.01).
        let leaky = ActivationConfig {
            negative_slope: Some(0.01),
            ..simple(ActivationKind::LeakyRelu)
        };
        assert_eq!(l.activations, vec![leaky; 23]);

        // gating_mode array of "none": no secondary activations.
        assert_eq!(l.gating_modes, vec![GatingMode::None; 23]);
        assert_eq!(l.secondary_activations, vec![None; 23]);

        // "slimmable": null on these submodels means absent.
        assert_eq!(l.slimmable, None);

        assert_eq!(cfg.head, None);
        assert_eq!(cfg.head_scale, Some(0.01));
    }
}

// -- A1 equivalence -----------------------------------------------------------

/// A plain A1 layer-array config (mirrors real A1 exports).
fn plain_a1_config() -> Value {
    json!({
        "layers": [
            {
                "input_size": 1, "condition_size": 1, "head_size": 1,
                "channels": 16, "kernel_size": 3, "dilations": [1, 2, 4, 8],
                "activation": "Tanh", "gated": false, "head_bias": false
            },
            {
                "input_size": 16, "condition_size": 1, "head_size": 1,
                "channels": 8, "kernel_size": 3, "dilations": [16, 32],
                "activation": "Tanh", "gated": false, "head_bias": true
            }
        ],
        "head": null,
        "head_scale": 0.02
    })
}

#[test]
fn plain_a1_config_parses_to_a1_equivalent_defaults() {
    let cfg = parse_full_wavenet_config(&plain_a1_config()).unwrap();

    assert_eq!(cfg.in_channels, 1);
    assert_eq!(cfg.head, None);
    assert_eq!(cfg.head_scale, Some(0.02));
    assert_eq!(cfg.condition_dsp, None);

    assert_eq!(cfg.layer_arrays.len(), 2);
    let l0 = &cfg.layer_arrays[0];
    assert_eq!(l0.input_size, 1);
    assert_eq!(l0.condition_size, 1);
    assert_eq!(l0.channels, 16);
    assert_eq!(l0.dilations, vec![1, 2, 4, 8]);
    assert_eq!(l0.kernel_sizes, vec![3, 3, 3, 3]);
    assert_eq!(l0.head_size, 1);
    assert!(!l0.head_bias);
    assert_eq!(l0.activations, vec![simple(ActivationKind::Tanh); 4]);

    // Every A2 extension sits at its A1-equivalent default.
    for l in &cfg.layer_arrays {
        assert_eq!(l.bottleneck, l.channels);
        assert!(l.gating_modes.iter().all(|m| *m == GatingMode::None));
        assert!(l.secondary_activations.iter().all(Option::is_none));
        assert_eq!(l.groups_input, 1);
        assert_eq!(l.groups_input_mixin, 1);
        assert_eq!(l.layer1x1, Layer1x1Params::default());
        assert_eq!(
            l.head1x1,
            Head1x1Params {
                active: false,
                out_channels: l.channels,
                groups: 1
            }
        );
        for f in [
            l.conv_pre_film,
            l.conv_post_film,
            l.input_mixin_pre_film,
            l.input_mixin_post_film,
            l.activation_pre_film,
            l.activation_post_film,
            l.layer1x1_post_film,
            l.head1x1_post_film,
        ] {
            assert_eq!(f, FilmParams::default());
        }
        assert_eq!(l.head_kernel_size, 1);
        assert_eq!(l.head_dilation, 1);
        assert_eq!(l.slimmable, None);
    }

    assert!(cfg.layer_arrays[1].head_bias);
}

#[test]
fn a1_gated_boolean_defaults_secondary_to_sigmoid() {
    let mut cfg_json = plain_a1_config();
    cfg_json["layers"][0]["gated"] = json!(true);
    cfg_json["layers"][1]["gated"] = json!(true);
    let cfg = parse_full_wavenet_config(&cfg_json).unwrap();
    for l in &cfg.layer_arrays {
        assert!(l.gating_modes.iter().all(|m| *m == GatingMode::Gated));
        assert!(l
            .secondary_activations
            .iter()
            .all(|s| *s == Some(simple(ActivationKind::Sigmoid))));
    }
}

#[test]
fn a1_lenient_defaults_kernel_activation_head_bias() {
    // Old exports may omit kernel_size, activation, and head_bias; the
    // engine's current parser defaults them to 2 / Tanh / true.
    let cfg = parse_full_wavenet_config(&json!({
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 4, "dilations": [1, 2]
        }]
    }))
    .unwrap();
    let l = &cfg.layer_arrays[0];
    assert_eq!(l.kernel_sizes, vec![2, 2]);
    assert_eq!(l.activations, vec![simple(ActivationKind::Tanh); 2]);
    assert!(l.head_bias);
    assert_eq!(cfg.head_scale, None);
}

// -- Handcrafted A2 snippets --------------------------------------------------

fn minimal_a2_layer() -> Value {
    json!({
        "input_size": 1, "condition_size": 1, "head_size": 1,
        "channels": 4, "kernel_size": 2, "dilations": [1, 2],
        "activation": {"type": "LeakyReLU", "negative_slope": 0.2},
        "head_bias": false
    })
}

fn config_with_layer(layer: Value) -> Value {
    json!({"layers": [layer], "head": null, "head_scale": 0.02})
}

#[test]
fn activation_object_parses_parameters() {
    let cfg = parse_full_wavenet_config(&config_with_layer(minimal_a2_layer())).unwrap();
    let expected = ActivationConfig {
        negative_slope: Some(0.2),
        ..simple(ActivationKind::LeakyRelu)
    };
    assert_eq!(cfg.layer_arrays[0].activations, vec![expected; 2]);
}

#[test]
fn blended_gating_with_single_secondary_activation() {
    let mut layer = minimal_a2_layer();
    layer["gating_mode"] = json!("blended");
    layer["secondary_activation"] = json!({"type": "LeakyReLU", "negative_slope": 0.3});
    let cfg = parse_full_wavenet_config(&config_with_layer(layer)).unwrap();
    let l = &cfg.layer_arrays[0];
    assert_eq!(l.gating_modes, vec![GatingMode::Blended; 2]);
    let expected = ActivationConfig {
        negative_slope: Some(0.3),
        ..simple(ActivationKind::LeakyRelu)
    };
    assert_eq!(l.secondary_activations, vec![Some(expected); 2]);
}

#[test]
fn blended_gating_without_secondary_defaults_to_sigmoid() {
    let mut layer = minimal_a2_layer();
    layer["gating_mode"] = json!("blended");
    let cfg = parse_full_wavenet_config(&config_with_layer(layer)).unwrap();
    assert_eq!(
        cfg.layer_arrays[0].secondary_activations,
        vec![Some(simple(ActivationKind::Sigmoid)); 2]
    );
}

#[test]
fn film_block_defaults_and_literal_false() {
    let mut layer = minimal_a2_layer();
    // Partial object: active/shift default true, groups defaults to 1.
    layer["conv_pre_film"] = json!({});
    layer["conv_post_film"] = json!({"active": true, "shift": false, "groups": 2});
    layer["activation_pre_film"] = json!(false);
    let cfg = parse_full_wavenet_config(&config_with_layer(layer)).unwrap();
    let l = &cfg.layer_arrays[0];
    assert_eq!(l.conv_pre_film, film(true, true, 1));
    assert_eq!(l.conv_post_film, film(true, false, 2));
    assert_eq!(l.activation_pre_film, FilmParams::default());
}

#[test]
fn layer_head_object_with_dilation() {
    let mut layer = minimal_a2_layer();
    let obj = layer.as_object_mut().unwrap();
    obj.remove("head_size");
    obj.remove("head_bias");
    obj.insert(
        "head".into(),
        json!({"out_channels": 2, "kernel_size": 16, "head_dilation": 4, "bias": true}),
    );
    let cfg = parse_full_wavenet_config(&config_with_layer(layer)).unwrap();
    let l = &cfg.layer_arrays[0];
    assert_eq!(l.head_size, 2);
    assert_eq!(l.head_kernel_size, 16);
    assert_eq!(l.head_dilation, 4);
    assert!(l.head_bias);
}

#[test]
fn slimmable_without_allowed_channels_defaults_to_full_range() {
    let mut layer = minimal_a2_layer();
    layer["slimmable"] = json!({"method": "slice_channels_uniform"});
    let cfg = parse_full_wavenet_config(&config_with_layer(layer)).unwrap();
    assert_eq!(
        cfg.layer_arrays[0]
            .slimmable
            .as_ref()
            .unwrap()
            .allowed_channels,
        vec![1, 2, 3, 4]
    );
}

#[test]
fn windowed_top_level_head_parses() {
    let mut config = config_with_layer(minimal_a2_layer());
    config["head"] = json!({
        "channels": 8, "out_channels": 1, "kernel_sizes": [2, 2],
        "activation": {"type": "LeakyReLU", "negative_slope": 0.1}
    });
    let cfg = parse_full_wavenet_config(&config).unwrap();
    let expected_act = ActivationConfig {
        negative_slope: Some(0.1),
        ..simple(ActivationKind::LeakyRelu)
    };
    assert_eq!(
        cfg.head,
        Some(HeadParams::Windowed {
            in_channels: 1, // implied by the layer array's head_size
            channels: 8,
            out_channels: 1,
            kernel_sizes: vec![2, 2],
            activation: expected_act,
        })
    );
}

#[test]
fn legacy_mlp_head_parses() {
    let mut config = config_with_layer(minimal_a2_layer());
    config["head"] = json!({"channels": 8, "num_layers": 1, "out_channels": 1});
    let cfg = parse_full_wavenet_config(&config).unwrap();
    assert_eq!(
        cfg.head,
        Some(HeadParams::Mlp {
            channels: 8,
            num_layers: 1,
            out_channels: 1
        })
    );
}

// -- Error cases --------------------------------------------------------------

fn parse_layer_err(layer: Value) -> String {
    parse_full_wavenet_config(&config_with_layer(layer)).unwrap_err()
}

#[test]
fn kernel_size_and_kernel_sizes_together_error() {
    let mut layer = minimal_a2_layer();
    layer["kernel_sizes"] = json!([2, 2]);
    assert!(parse_layer_err(layer).contains("only one of kernel_size"));
}

#[test]
fn kernel_sizes_length_mismatch_errors() {
    let mut layer = minimal_a2_layer();
    layer.as_object_mut().unwrap().remove("kernel_size");
    layer["kernel_sizes"] = json!([2, 2, 2]);
    assert!(parse_layer_err(layer).contains("must match dilations size"));
}

#[test]
fn activation_array_length_mismatch_errors() {
    let mut layer = minimal_a2_layer();
    layer["activation"] = json!(["Tanh"]);
    assert!(parse_layer_err(layer).contains("activation array size"));
}

#[test]
fn gating_mode_array_length_mismatch_errors() {
    let mut layer = minimal_a2_layer();
    layer["gating_mode"] = json!(["gated"]);
    assert!(parse_layer_err(layer).contains("gating_mode array size"));
}

#[test]
fn unknown_gating_mode_errors() {
    let mut layer = minimal_a2_layer();
    layer["gating_mode"] = json!("sideways");
    assert!(parse_layer_err(layer).contains("Invalid gating_mode: sideways"));
}

#[test]
fn secondary_activation_array_shorter_than_layers_errors() {
    let mut layer = minimal_a2_layer();
    layer["gating_mode"] = json!(["gated", "gated"]);
    layer["secondary_activation"] = json!(["Sigmoid"]);
    assert!(parse_layer_err(layer).contains("secondary_activation array size"));
}

#[test]
fn layer1x1_post_film_without_layer1x1_errors() {
    let mut layer = minimal_a2_layer();
    layer["layer1x1"] = json!({"active": false, "groups": 1});
    layer["layer1x1_post_film"] = json!({"active": true, "shift": true, "groups": 1});
    assert!(parse_layer_err(layer)
        .contains("layer1x1_post_film cannot be active when layer1x1.active is false"));
}

#[test]
fn zero_head_kernel_size_errors() {
    let mut layer = minimal_a2_layer();
    let obj = layer.as_object_mut().unwrap();
    obj.remove("head_size");
    obj.insert(
        "head".into(),
        json!({"out_channels": 1, "kernel_size": 0, "bias": false}),
    );
    assert!(parse_layer_err(layer).contains("head.kernel_size must be >= 1"));
}

#[test]
fn unsupported_slimmable_method_errors() {
    let mut layer = minimal_a2_layer();
    layer["slimmable"] = json!({"method": "magic_shrink"});
    assert!(parse_layer_err(layer).contains("unsupported slimmable method 'magic_shrink'"));
}

#[test]
fn slimmable_last_entry_must_be_full_channel_count() {
    let mut layer = minimal_a2_layer();
    layer["slimmable"] = json!({
        "method": "slice_channels_uniform",
        "kwargs": {"allowed_channels": [1, 2]}
    });
    assert!(parse_layer_err(layer).contains("must equal the full channel count"));
}

#[test]
fn missing_layer_head_fields_error() {
    let mut layer = minimal_a2_layer();
    layer.as_object_mut().unwrap().remove("head_size");
    assert!(parse_layer_err(layer).contains("expected 'head' object"));
}

#[test]
fn empty_layers_array_errors() {
    let err = parse_full_wavenet_config(&json!({"layers": [], "head_scale": 0.02})).unwrap_err();
    assert!(err.contains("at least one layer array"));
}

#[test]
fn windowed_head_in_channels_mismatch_errors() {
    let mut config = config_with_layer(minimal_a2_layer());
    config["head"] = json!({
        "in_channels": 5, "channels": 8, "out_channels": 1,
        "kernel_sizes": [2], "activation": "Tanh"
    });
    let err = parse_full_wavenet_config(&config).unwrap_err();
    assert!(err.contains("must equal last layer array's head_size"));
}

// Direct struct-level sanity: WaveNetFullConfig is plain data.
#[test]
fn full_config_is_cloneable_and_comparable() {
    let cfg = parse_full_wavenet_config(&config_with_layer(minimal_a2_layer())).unwrap();
    let clone: WaveNetFullConfig = cfg.clone();
    assert_eq!(cfg, clone);
}
