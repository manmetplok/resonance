//! Translation of a raw WaveNet `config` JSON object into the engine's
//! [`WaveNetConfig`].
//!
//! # Which surface owns which key
//!
//! New-format (layer-array) configs are read **only** by the typed A2
//! surface, [`WaveNetFullConfig`] in [`super::super::wavenet::params`]. This
//! module never reaches into a layer object itself; it maps the typed
//! `LayerArrayParams` onto [`StackConfig`] field by field, so a key the typed
//! surface honours cannot be dropped on the way to inference (and a key it
//! rejects cannot reach the engine).
//!
//! Two exceptions, both deliberate:
//!
//! * the flat A1 config (`schema::OldWaveNetConfig`) has no typed
//!   counterpart — the typed surface requires a `layers` array of objects —
//!   so it is translated here from its own serde mirror;
//! * [`config_has_a2_markers`] and the `condition_dsp` rate walk read the raw
//!   JSON, but only ask whether a key is *present* (activation flavor, nested
//!   sample rates); they never interpret a value the typed surface also
//!   interprets.

use super::super::activations::{ActivationConfig, ActivationKind};
use super::super::wavenet::params::{
    GatingMode, Head1x1Params, HeadParams, LayerArrayParams, LayerFilms, WaveNetFullConfig,
};
use super::engine_config::{StackConfig, WaveNetConfig};
use super::schema::{parse_sample_rate, OldWaveNetConfig};

/// Parse a WaveNet `config` JSON object (old flat or new layer-array format)
/// into the engine's [`WaveNetConfig`]. This is the exact parse
/// `load_model_from_file` uses before model construction; exposed so tests
/// can assert what reaches the engine for a given config.
/// [`WaveNetConfig::fast_activations`] is cleared by probing the raw JSON
/// for A2 markers ([`config_has_a2_markers`]); nested condition_dsp and
/// container submodels force exact activations regardless (they only exist
/// in the A2 era).
pub fn parse_wavenet_config(value: serde_json::Value) -> Result<WaveNetConfig, String> {
    // Try old format first (flat config with integer layer counts).
    if let Ok(old) = serde_json::from_value::<OldWaveNetConfig>(value.clone()) {
        return old.into_config();
    }
    let a2_surface = config_has_a2_markers(&value);
    // New format: the typed A2 surface is the only reader of these keys.
    let full =
        parse_full_wavenet_config(&value).map_err(|e| format!("Invalid WaveNet config: {e}"))?;
    engine_config_from_typed(full, a2_surface)
}

/// Parse a WaveNet config covering the full A2 surface into typed structs
/// (see [`super::super::wavenet::params`]): activation config
/// objects/arrays, `bottleneck`, blended gating + `secondary_activation`,
/// grouped convolutions, `layer1x1`/`head1x1` objects, the 8 FiLM insertion
/// points, windowed heads, `condition_dsp`, and `slimmable` descriptors.
/// Plain A1 layer-array configs parse too, with every A2 field at its
/// A1-equivalent default.
///
/// This is the single reader of new-format config JSON:
/// [`parse_wavenet_config`] translates the result into the engine config
/// rather than re-reading the JSON.
pub fn parse_full_wavenet_config(value: &serde_json::Value) -> Result<WaveNetFullConfig, String> {
    WaveNetFullConfig::from_json(value)
}

// -- Old (flat) NAM format ----------------------------------------------------

impl OldWaveNetConfig {
    pub(super) fn into_config(self) -> Result<WaveNetConfig, String> {
        let activation = ActivationConfig::from_name(&self.activation)?;
        // Layer counts become dilations 1, 2, ..., 2^(n-1); a hostile
        // count would overflow the shift below (real models use ~10 per
        // stack). Counts that pass here but are still absurd fall to the
        // state-buffer bound at model construction.
        if let Some(&n) = self.layers.iter().find(|&&n| n >= usize::BITS as usize) {
            return Err(format!(
                "WaveNet config: {n} layers in a stack is not supported"
            ));
        }
        let dilations: Vec<Vec<usize>> = self
            .layers
            .iter()
            .map(|&n| (0..n).map(|i| 1usize << i).collect())
            .collect();
        let stacks = dilations
            .into_iter()
            .map(|d: Vec<usize>| {
                let n = d.len();
                // Old-format gating is a plain boolean: every layer is
                // "gated" with the backward-compat Sigmoid secondary (which
                // resolves to the fast sigmoid — today's A1 path).
                let (gating_modes, secondary_activations) = if self.gated {
                    (
                        vec![GatingMode::Gated; n],
                        vec![Some(ActivationConfig::simple(ActivationKind::Sigmoid)); n],
                    )
                } else {
                    (vec![GatingMode::None; n], vec![None; n])
                };
                StackConfig {
                    input_size: self.input_size,
                    condition_size: self.condition_size,
                    head_size: self.head_size,
                    // Old-format models predate the windowed head rechannel.
                    head_kernel_size: 1,
                    head_dilation: 1,
                    head_bias: self.head_bias,
                    channels: self.channels,
                    // Old-format models predate the A2 bottleneck.
                    bottleneck: self.channels,
                    kernel_sizes: vec![2; n],
                    dilations: d,
                    activations: vec![activation.clone(); n],
                    gating_modes,
                    secondary_activations,
                    // Old-format models predate the layer1x1 residual conv
                    // entirely (config-wide `has_layer1x1: false` governs;
                    // the per-stack flag stays at its neutral default).
                    layer1x1_active: true,
                    // Old-format models predate grouped convolutions.
                    groups_input: 1,
                    groups_input_mixin: 1,
                    layer1x1_groups: 1,
                    // Old-format models predate the head1x1 skip conv.
                    head1x1: Head1x1Params::inactive(self.channels),
                    // Old-format models predate FiLM conditioning.
                    films: LayerFilms::default(),
                }
            })
            .collect();
        Ok(WaveNetConfig {
            input_size: self.input_size,
            stacks,
            head: self.head,
            head_size: self.head_size,
            has_layer1x1: false,
            // Old-format models predate the condition_dsp sub-network.
            condition_dsp: None,
            // Old flat configs cannot carry A2 markers: A1 flavor.
            fast_activations: true,
        })
    }
}

// -- Typed A2 config -> engine config ------------------------------------------

/// Cap on legacy head-MLP hidden layers (real exports use 1-2); the count
/// sizes an allocation directly, so it cannot wait for the weight-derived
/// bounds at model construction.
const MAX_HEAD_MLP_LAYERS: usize = 64;

/// Flatten the typed config onto the engine's per-stack shape. Every field
/// is a straight copy; the only decisions taken here are engine-side
/// capability limits (which post-stack heads the engine can run) and the
/// activation flavor.
fn engine_config_from_typed(
    full: WaveNetFullConfig,
    a2_surface: bool,
) -> Result<WaveNetConfig, String> {
    // `WaveNetFullConfig::from_json` rejects an empty `layers` array, so
    // there is always a first stack to take the model input size from.
    let first_input_size = full
        .layer_arrays
        .first()
        .ok_or("WaveNet config has no layer arrays")?
        .input_size;
    let stacks: Vec<StackConfig> = full
        .layer_arrays
        .into_iter()
        .map(stack_from_typed)
        .collect();

    let (head, head_size) = match full.head {
        // The reference A2 post-stack head (`detail::Head`: per-conv
        // kernel_sizes with activations applied ahead of every conv) is
        // structurally different from the engine's legacy head-MLP; running
        // an A2 file through the latter would silently produce wrong audio,
        // so any head on an A2-marked config is rejected. Pre-A2 (A1-shaped)
        // configs keep the historical head-MLP path.
        Some(_) if a2_surface => {
            return Err(
                "WaveNet config: a post-stack 'head' is not supported for A2 models".to_string(),
            );
        }
        Some(HeadParams::Mlp {
            channels,
            num_layers,
            out_channels,
        }) => {
            // `num_layers` sizes an allocation straight from a parsed
            // field; real head MLPs have one or two hidden layers, so a
            // huge count is a hostile or corrupt file, not a model.
            if num_layers > MAX_HEAD_MLP_LAYERS {
                return Err(format!(
                    "WaveNet config: head num_layers ({num_layers}) exceeds the supported maximum ({MAX_HEAD_MLP_LAYERS})"
                ));
            }
            let hidden = if num_layers > 0 {
                vec![channels; num_layers]
            } else {
                vec![]
            };
            (hidden, out_channels)
        }
        // A windowed head always carries `kernel_sizes`/`activation`, which
        // are A2 markers, so this is unreachable via `parse_wavenet_config`;
        // kept explicit so the engine never guesses at one.
        Some(HeadParams::Windowed { .. }) => {
            return Err(
                "WaveNet config: a windowed post-stack 'head' is not supported".to_string(),
            );
        }
        // stacks is non-empty (checked above via `first`).
        None => (vec![], stacks[0].head_size),
    };

    Ok(WaveNetConfig {
        input_size: first_input_size,
        stacks,
        head,
        head_size,
        has_layer1x1: true,
        condition_dsp: full.condition_dsp,
        fast_activations: !a2_surface,
    })
}

fn stack_from_typed(l: LayerArrayParams) -> StackConfig {
    StackConfig {
        input_size: l.input_size,
        condition_size: l.condition_size,
        head_size: l.head_size,
        head_kernel_size: l.head_kernel_size,
        head_dilation: l.head_dilation,
        head_bias: l.head_bias,
        channels: l.channels,
        bottleneck: l.bottleneck,
        layer1x1_active: l.layer1x1.active,
        dilations: l.dilations,
        kernel_sizes: l.kernel_sizes,
        activations: l.activations,
        gating_modes: l.gating_modes,
        secondary_activations: l.secondary_activations,
        groups_input: l.groups_input,
        groups_input_mixin: l.groups_input_mixin,
        layer1x1_groups: l.layer1x1.groups,
        head1x1: l.head1x1,
        films: LayerFilms {
            conv_pre: l.conv_pre_film,
            conv_post: l.conv_post_film,
            input_mixin_pre: l.input_mixin_pre_film,
            input_mixin_post: l.input_mixin_post_film,
            activation_pre: l.activation_pre_film,
            activation_post: l.activation_post_film,
            layer1x1_post: l.layer1x1_post_film,
            head1x1_post: l.head1x1_post_film,
        },
    }
}

// -- Raw-JSON probes ----------------------------------------------------------

/// The FiLM insertion-point keys of an A2 layer-array config (reference
/// site order).
const FILM_KEYS: [&str; 8] = [
    "conv_pre_film",
    "conv_post_film",
    "input_mixin_pre_film",
    "input_mixin_post_film",
    "activation_pre_film",
    "activation_post_film",
    "layer1x1_post_film",
    "head1x1_post_film",
];

/// Raw-JSON probe: does this (new-format) WaveNet config carry any A2
/// marker? Since ba todo #1116 every model runs the reference
/// (NeuralAmpModelerCore) forward-pass STRUCTURE; this gate only selects
/// the ACTIVATION FLAVOR ([`WaveNetConfig::fast_activations`]) — plus the
/// rejection of a post-stack `head` on A2-marked configs. A config keeps
/// the fast (A1) activations only when it is fully expressible in the
/// pre-A2 surface (plain string activations, `gated` booleans, flat
/// `head_size`/`head_bias`, `kernel_size`/`kernel_sizes`, no
/// grouped/bottleneck/FiLM/head1x1/layer1x1 objects, no `condition_dsp`,
/// no `slimmable`) — exactly the files the official NAM plugin runs with
/// `enable_fast_tanh()`. Anything the A2 era introduced flips the model to
/// the exact activations the A2 reference renders were produced with.
///
/// Deliberately NOT markers: `kernel_sizes` (per-layer kernels predate
/// A2), `gated`, and every field of the original layer-array format.
pub fn config_has_a2_markers(config: &serde_json::Value) -> bool {
    let non_null = |v: Option<&serde_json::Value>| v.is_some_and(|v| !v.is_null());
    if non_null(config.get("condition_dsp")) {
        return true;
    }
    // An A2-style post-stack head carries `kernel_sizes`/`activation`
    // (the legacy head-MLP shape does not).
    if let Some(head) = config.get("head") {
        if head.get("kernel_sizes").is_some() || head.get("activation").is_some() {
            return true;
        }
    }
    let Some(layers) = config.get("layers").and_then(serde_json::Value::as_array) else {
        return false;
    };
    layers.iter().any(|l| {
        non_null(l.get("bottleneck"))
            || non_null(l.get("gating_mode"))
            || non_null(l.get("secondary_activation"))
            || non_null(l.get("groups_input"))
            || non_null(l.get("groups_input_mixin"))
            || non_null(l.get("layer1x1"))
            || non_null(l.get("head1x1"))
            || non_null(l.get("head"))
            || non_null(l.get("slimmable"))
            // A2 activation config objects / per-layer arrays; a plain
            // string is the A1 surface.
            || l.get("activation")
                .is_some_and(|a| a.is_object() || a.is_array())
            || FILM_KEYS.iter().any(|k| non_null(l.get(k)))
    })
}

/// Walk a WaveNet config's `condition_dsp` chain and validate every nested
/// model's sample rate against the top-level model's (reference
/// `parse_config_json`, applied per level through the recursive `get_dsp`).
pub(super) fn check_condition_dsp_rates(
    config: &serde_json::Value,
    expected: f32,
) -> Result<(), String> {
    let Some(cd) = config.get("condition_dsp").filter(|v| !v.is_null()) else {
        return Ok(());
    };
    let nested_rate = parse_sample_rate(cd.get("sample_rate"));
    if nested_rate != expected {
        return Err(format!(
            "condition_dsp expected sample rate ({nested_rate}) doesn't match model sample rate ({expected})"
        ));
    }
    match cd.get("config") {
        Some(nested_config) => check_condition_dsp_rates(nested_config, expected),
        None => Ok(()),
    }
}
