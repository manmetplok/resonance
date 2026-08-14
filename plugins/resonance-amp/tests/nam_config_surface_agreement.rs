//! The engine config and the typed A2 config must agree, key for key.
//!
//! New-format (layer-array) WaveNet configs have exactly one reader — the
//! typed A2 surface, `wavenet::params::WaveNetFullConfig`. `parse_wavenet_config`
//! translates that typed config into the engine's `WaveNetConfig` instead of
//! re-reading the JSON, which is what stops an A2 key being honoured by one
//! path and silently ignored by the other (ba todo #1262).
//!
//! This suite pins that invariant on every fixture config in the repo,
//! including nested `condition_dsp` sub-models and container submodels: for
//! each layer array, every field the engine consumes must equal the typed
//! field it comes from. Should a second reader ever be reintroduced, any key
//! it handles differently shows up here.

use resonance_amp::nam::parse::{parse_full_wavenet_config, parse_wavenet_config, StackConfig};
use resonance_amp::nam::wavenet::params::LayerArrayParams;
use serde_json::Value;

mod common;
use common::fixture_path_in;

/// Every WaveNet `config` object reachable from a fixture file, labelled by
/// where it sits (nested condition_dsp nets and container submodels
/// included).
fn fixture_configs() -> Vec<(String, Value)> {
    let files = [
        ("a1", "wavenet.nam"),
        ("a1", "wavenet_a1_standard.nam"),
        ("a2", "wavenet_a2_max.nam"),
        ("a2", "wavenet_condition_dsp.nam"),
        ("a2", "slimmable_wavenet.nam"),
        ("a2", "A2.nam"),
    ];
    let mut out = Vec::new();
    for (dir, name) in files {
        let path = fixture_path_in(dir, name);
        let text = std::fs::read_to_string(&path).expect("fixture must be readable");
        let file: Value = serde_json::from_str(&text).expect("fixture must be JSON");
        collect_models(name, &file, &mut out);
    }
    assert!(
        out.len() >= 8,
        "expected every fixture model to be collected, got {}",
        out.len()
    );
    out
}

/// Walk one `.nam`-style model object, collecting its WaveNet config and
/// recursing into `condition_dsp` sub-models and container submodels.
fn collect_models(label: &str, model: &Value, out: &mut Vec<(String, Value)>) {
    let architecture = model["architecture"].as_str().unwrap_or_default();
    let config = &model["config"];
    match architecture {
        "WaveNet" => {
            out.push((label.to_string(), config.clone()));
            if let Some(cd) = config.get("condition_dsp").filter(|v| !v.is_null()) {
                collect_models(&format!("{label}/condition_dsp"), cd, out);
            }
        }
        "SlimmableContainer" => {
            let submodels = config["submodels"]
                .as_array()
                .expect("container submodels must be an array");
            for (i, entry) in submodels.iter().enumerate() {
                collect_models(&format!("{label}/submodel{i}"), &entry["model"], out);
            }
        }
        other => panic!("{label}: unexpected fixture architecture {other}"),
    }
}

/// Assert one engine stack carries exactly what the typed layer array says.
fn assert_stack_matches(label: &str, stack: &StackConfig, typed: &LayerArrayParams) {
    assert_eq!(stack.input_size, typed.input_size, "{label}: input_size");
    assert_eq!(
        stack.condition_size, typed.condition_size,
        "{label}: condition_size"
    );
    assert_eq!(stack.channels, typed.channels, "{label}: channels");
    assert_eq!(stack.bottleneck, typed.bottleneck, "{label}: bottleneck");
    assert_eq!(stack.head_size, typed.head_size, "{label}: head_size");
    assert_eq!(
        stack.head_kernel_size, typed.head_kernel_size,
        "{label}: head_kernel_size"
    );
    assert_eq!(
        stack.head_dilation, typed.head_dilation,
        "{label}: head_dilation"
    );
    assert_eq!(stack.head_bias, typed.head_bias, "{label}: head_bias");
    assert_eq!(stack.dilations, typed.dilations, "{label}: dilations");
    assert_eq!(
        stack.kernel_sizes, typed.kernel_sizes,
        "{label}: kernel_sizes"
    );
    assert_eq!(stack.activations, typed.activations, "{label}: activations");
    assert_eq!(
        stack.gating_modes, typed.gating_modes,
        "{label}: gating_modes"
    );
    assert_eq!(
        stack.secondary_activations, typed.secondary_activations,
        "{label}: secondary_activations"
    );
    assert_eq!(
        stack.groups_input, typed.groups_input,
        "{label}: groups_input"
    );
    assert_eq!(
        stack.groups_input_mixin, typed.groups_input_mixin,
        "{label}: groups_input_mixin"
    );
    assert_eq!(
        stack.layer1x1_active, typed.layer1x1.active,
        "{label}: layer1x1.active"
    );
    assert_eq!(
        stack.layer1x1_groups, typed.layer1x1.groups,
        "{label}: layer1x1.groups"
    );
    assert_eq!(stack.head1x1, typed.head1x1, "{label}: head1x1");
    // The 8 FiLM insertion points, in reference site order.
    assert_eq!(
        stack.films.conv_pre, typed.conv_pre_film,
        "{label}: conv_pre_film"
    );
    assert_eq!(
        stack.films.conv_post, typed.conv_post_film,
        "{label}: conv_post_film"
    );
    assert_eq!(
        stack.films.input_mixin_pre, typed.input_mixin_pre_film,
        "{label}: input_mixin_pre_film"
    );
    assert_eq!(
        stack.films.input_mixin_post, typed.input_mixin_post_film,
        "{label}: input_mixin_post_film"
    );
    assert_eq!(
        stack.films.activation_pre, typed.activation_pre_film,
        "{label}: activation_pre_film"
    );
    assert_eq!(
        stack.films.activation_post, typed.activation_post_film,
        "{label}: activation_post_film"
    );
    assert_eq!(
        stack.films.layer1x1_post, typed.layer1x1_post_film,
        "{label}: layer1x1_post_film"
    );
    assert_eq!(
        stack.films.head1x1_post, typed.head1x1_post_film,
        "{label}: head1x1_post_film"
    );
}

#[test]
fn every_fixture_config_reaches_the_engine_exactly_as_typed() {
    for (label, config) in fixture_configs() {
        let typed = parse_full_wavenet_config(&config)
            .unwrap_or_else(|e| panic!("{label}: typed parse failed: {e}"));
        let engine = parse_wavenet_config(config.clone())
            .unwrap_or_else(|e| panic!("{label}: engine parse failed: {e}"));
        assert_eq!(
            engine.stacks.len(),
            typed.layer_arrays.len(),
            "{label}: stack count"
        );
        for (i, (stack, arr)) in engine
            .stacks
            .iter()
            .zip(typed.layer_arrays.iter())
            .enumerate()
        {
            assert_stack_matches(&format!("{label} layer {i}"), stack, arr);
        }
        // Model-level fields the engine derives from the typed config.
        assert_eq!(
            engine.input_size, typed.layer_arrays[0].input_size,
            "{label}: model input_size"
        );
        assert_eq!(
            engine.condition_dsp, typed.condition_dsp,
            "{label}: condition_dsp"
        );
        assert!(engine.has_layer1x1, "{label}: new-format layer1x1 flag");
    }
}

/// The legacy MLP head is the one post-stack head the engine can run, and
/// it must arrive with the typed shape (hidden layers duplicated, output
/// size taken from the head rather than the last stack).
#[test]
fn legacy_mlp_head_translates_to_the_engine_head() {
    let config = serde_json::json!({
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 4,
            "channels": 2, "dilations": [1], "kernel_size": 1,
            "activation": "Tanh", "gated": false, "head_bias": true
        }],
        "head": {"channels": 8, "num_layers": 2, "out_channels": 1},
        "head_scale": 0.02
    });
    let engine = parse_wavenet_config(config).expect("legacy MLP head must parse");
    assert_eq!(engine.head, vec![8, 8]);
    assert_eq!(engine.head_size, 1);

    // No head: the engine's output size follows the first stack's head.
    let config = serde_json::json!({
        "layers": [{
            "input_size": 1, "condition_size": 1, "head_size": 4,
            "channels": 2, "dilations": [1], "kernel_size": 1,
            "activation": "Tanh", "gated": false, "head_bias": true
        }],
        "head": null,
        "head_scale": 0.02
    });
    let engine = parse_wavenet_config(config).expect("head-less config must parse");
    assert!(engine.head.is_empty());
    assert_eq!(engine.head_size, 4);
}
