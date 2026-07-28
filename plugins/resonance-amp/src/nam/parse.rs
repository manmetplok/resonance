/// NAM .nam file parsing and model construction.
use serde::Deserialize;
use std::path::Path;

use super::activations::{ActivationConfig, ActivationKind};
use super::lstm::LstmModel;
use super::wavenet;
use super::wavenet::params::{
    parse_activation_value, parse_gating_config, FilmParams, GatingMode, Head1x1Params, LayerFilms,
};
use super::wavenet::WaveNetModel;
use super::NamInference;

#[derive(Deserialize)]
struct NamFile {
    #[allow(dead_code)]
    version: Option<String>,
    architecture: String,
    config: serde_json::Value,
    weights: Vec<f32>,
    /// Rate the profile was captured/trained at. Kept as a raw JSON value
    /// because exporters write it as int, float, or (rarely) a string.
    #[serde(default)]
    sample_rate: Option<serde_json::Value>,
}

/// Sample rate assumed when a .nam file omits the `sample_rate` field.
/// Older NAM exporters didn't write it; the NAM convention is that such
/// profiles were captured at 48 kHz.
pub const DEFAULT_SAMPLE_RATE: f32 = 48_000.0;

/// A parsed NAM model plus the sample rate it expects to run at.
pub struct LoadedModel {
    pub model: Box<dyn NamInference>,
    pub sample_rate: f32,
}

fn parse_sample_rate(value: Option<&serde_json::Value>) -> f32 {
    value
        .and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        })
        .map(|r| r as f32)
        .filter(|r| *r > 0.0)
        .unwrap_or(DEFAULT_SAMPLE_RATE)
}

/// Internal WaveNet config used by the inference engine.
pub struct WaveNetConfig {
    pub input_size: usize,
    /// Per-stack config.
    pub stacks: Vec<StackConfig>,
    /// Head hidden layer sizes (e.g. [8]). Empty if no head MLP.
    pub head: Vec<usize>,
    /// Final output size from head (typically 1).
    pub head_size: usize,
    /// Whether layers have a learned 1x1 residual conv (_layer1x1).
    /// True for new-format NAM models (default), false for old format.
    pub has_layer1x1: bool,
    /// Raw A2 `condition_dsp` sub-model JSON: a complete nested .nam-style
    /// model object (`architecture`/`config`/`weights`/`sample_rate`) that
    /// preprocesses the raw model input into the condition signal fed to
    /// every layer array's input mixin and FiLM points. Constructed
    /// recursively at model-build time (see
    /// [`build_condition_dsp`]); `None` (A1 and condition_dsp-less A2)
    /// leaves the condition source untouched.
    pub condition_dsp: Option<serde_json::Value>,
}

pub struct StackConfig {
    pub input_size: usize,
    pub condition_size: usize,
    pub head_size: usize,
    /// Kernel size of this stack's head rechannel convolution (reference
    /// `LayerArrayParams::head_kernel_size`, from the A2 nested `head`
    /// object's `kernel_size`). A1 / legacy flat configs: 1, which is the
    /// historical 1x1 head rechannel exactly.
    pub head_kernel_size: usize,
    /// Dilation of the head rechannel convolution (`head.head_dilation`).
    /// A1: 1. Irrelevant when `head_kernel_size == 1`.
    pub head_dilation: usize,
    /// Whether this stack's head rechannel has a bias (per-array in the
    /// reference: nested `head.bias`, or legacy flat `head_bias`).
    pub head_bias: bool,
    pub channels: usize,
    /// Internal (bottleneck) channel count of each layer in this stack
    /// (A2). The dilated conv and input mixin output `bottleneck` channels
    /// (doubled when gated), activation runs at bottleneck width, and the
    /// layer1x1 maps bottleneck back to `channels`. A1 models — and A2
    /// models that omit the field — use `bottleneck == channels`, which
    /// degenerates to the historical layout exactly.
    pub bottleneck: usize,
    pub dilations: Vec<usize>,
    pub kernel_sizes: Vec<usize>,
    /// Layer activation(s) for this stack: a single entry is broadcast to
    /// every layer (A1 string configs and A2 single-object configs), or one
    /// entry per layer (A2 per-layer activation arrays). A1 models use
    /// `"Tanh"`, which resolves to the fast-tanh path at model
    /// construction.
    pub activations: Vec<ActivationConfig>,
    /// Gating mode per layer (same length as `dilations`; A2 allows mixed
    /// per-layer modes). Layers with a non-`None` mode have their conv and
    /// input-mixin output width doubled (primary + secondary halves).
    pub gating_modes: Vec<GatingMode>,
    /// Secondary (gate/blend) activation config per layer. `None` for
    /// ungated layers; a gated/blended layer without an explicit config
    /// defaults to `Sigmoid` (reference backward-compat), which the engine
    /// resolves to the fast sigmoid so A1 gated models stay bit-identical.
    pub secondary_activations: Vec<Option<ActivationConfig>>,
    /// Groups of the dilated input convolution (reference
    /// `LayerArrayParams::groups_input`). Splits the conv's `channels`
    /// inputs and `bottleneck`-wide (doubled when gated) outputs into
    /// contiguous per-group blocks. A1: 1.
    pub groups_input: usize,
    /// Groups of the condition input-mixin 1x1 convolution (reference
    /// `groups_input_mixin`). A1: 1.
    pub groups_input_mixin: usize,
    /// Groups of the layer1x1 residual convolution (reference
    /// `Layer1x1Params::groups`); only meaningful when the config has a
    /// layer1x1. A1: 1.
    pub layer1x1_groups: usize,
    /// Optional head1x1 skip-path convolution (reference `Head1x1Params`).
    /// When active, each layer's skip contribution is
    /// `head1x1(activated z)` (bottleneck -> `out_channels`, grouped) and
    /// the stack's skip accumulator / head_rechannel input are
    /// `out_channels` wide instead of `bottleneck`. A1: inactive.
    pub head1x1: Head1x1Params,
    /// The 8 per-layer FiLM insertion points (reference `_FiLMParams`
    /// members of `LayerArrayParams`). A1: all inactive, which leaves the
    /// signal untouched at every site.
    pub films: LayerFilms,
}

// -- Old NAM format (flat config with layer counts) --------------------------

#[derive(Deserialize)]
struct OldWaveNetConfig {
    input_size: usize,
    condition_size: usize,
    head_size: usize,
    channels: usize,
    layers: Vec<usize>,
    head: Vec<usize>,
    activation: String,
    gated: bool,
    head_bias: bool,
}

impl OldWaveNetConfig {
    fn into_config(self) -> Result<WaveNetConfig, String> {
        let activation = ActivationConfig::from_name(&self.activation)?;
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
        })
    }
}

// -- New NAM format (explicit layer array configs) ---------------------------

#[derive(Deserialize)]
struct NewLayerArrayConfig {
    input_size: usize,
    condition_size: usize,
    /// Legacy flat head output size (implicit kernel 1). The A2 trainer
    /// export uses the nested `head` object instead; the reference
    /// (`parse_config_json` in NAM/wavenet/model.cpp) prefers `head` when
    /// both are present and errors when both are absent.
    #[serde(default)]
    head_size: Option<usize>,
    /// A2 nested head rechannel config (`head.out_channels` /
    /// `kernel_size` / `head_dilation` / `bias`).
    #[serde(default)]
    head: Option<NewLayerHeadConfig>,
    channels: usize,
    /// Internal channel count (A2). Defaults to `channels` (A1), matching
    /// the reference (`bottleneck = layer_config.value("bottleneck",
    /// channels)` in NAM/wavenet/model.cpp).
    #[serde(default)]
    bottleneck: Option<usize>,
    dilations: Vec<usize>,
    #[serde(default)]
    kernel_size: Option<usize>,
    #[serde(default)]
    kernel_sizes: Option<Vec<usize>>,
    #[serde(default)]
    gated: Option<bool>,
    #[serde(default)]
    gating_mode: Option<serde_json::Value>,
    /// Secondary (gate/blend) activation: single config or per-layer array.
    #[serde(default)]
    secondary_activation: Option<serde_json::Value>,
    /// Activation: plain string (A1), A2-style config object, or a
    /// per-layer array of either. Absent in some old exports; defaults to
    /// `"Tanh"`.
    #[serde(default)]
    activation: Option<serde_json::Value>,
    #[serde(default = "default_true")]
    head_bias: bool,
    /// Groups of the dilated input conv (A2). Absent (A1) means 1.
    #[serde(default = "default_one")]
    groups_input: usize,
    /// Groups of the condition input-mixin conv (A2). Absent (A1) means 1.
    #[serde(default = "default_one")]
    groups_input_mixin: usize,
    /// A2 `layer1x1` object. Only its `groups` field feeds the engine here;
    /// existence of the residual 1x1 stays governed by the config-wide
    /// `has_layer1x1` (per-array `active` handling is typed-params scope,
    /// see [`super::wavenet::params::Layer1x1Params`]).
    #[serde(default)]
    layer1x1: Option<NewLayer1x1Config>,
    /// A2 `head1x1` object: optional 1x1 conv on the skip path. Absent (A1)
    /// means inactive with `out_channels = channels`, matching the reference
    /// `parse_config_json` fallback.
    #[serde(default)]
    head1x1: Option<NewHead1x1Config>,
    /// The 8 A2 FiLM insertion-point blocks. Kept as raw JSON values so the
    /// typed `FilmParams::from_json` semantics (absent/`null`/`false` =
    /// inactive; object defaults `{active: true, shift: true, groups: 1}`)
    /// stay single-sourced with the typed A2 parser.
    #[serde(default)]
    conv_pre_film: Option<serde_json::Value>,
    #[serde(default)]
    conv_post_film: Option<serde_json::Value>,
    #[serde(default)]
    input_mixin_pre_film: Option<serde_json::Value>,
    #[serde(default)]
    input_mixin_post_film: Option<serde_json::Value>,
    #[serde(default)]
    activation_pre_film: Option<serde_json::Value>,
    #[serde(default)]
    activation_post_film: Option<serde_json::Value>,
    #[serde(default)]
    layer1x1_post_film: Option<serde_json::Value>,
    #[serde(default)]
    head1x1_post_film: Option<serde_json::Value>,
}

impl NewLayerArrayConfig {
    /// Parse the 8 FiLM insertion-point blocks (reference weight/site order).
    fn films(&self, ctx: &str) -> Result<LayerFilms, String> {
        let film = |v: &Option<serde_json::Value>, key: &str| {
            FilmParams::from_json(v.as_ref().filter(|v| !v.is_null()), ctx, key)
        };
        Ok(LayerFilms {
            conv_pre: film(&self.conv_pre_film, "conv_pre_film")?,
            conv_post: film(&self.conv_post_film, "conv_post_film")?,
            input_mixin_pre: film(&self.input_mixin_pre_film, "input_mixin_pre_film")?,
            input_mixin_post: film(&self.input_mixin_post_film, "input_mixin_post_film")?,
            activation_pre: film(&self.activation_pre_film, "activation_pre_film")?,
            activation_post: film(&self.activation_post_film, "activation_post_film")?,
            layer1x1_post: film(&self.layer1x1_post_film, "layer1x1_post_film")?,
            head1x1_post: film(&self.head1x1_post_film, "head1x1_post_film")?,
        })
    }
}

/// A2 per-array head rechannel object. `out_channels`, `kernel_size`, and
/// `bias` are required when the object is present (reference
/// `head_json.at(...)` throws on absence); `head_dilation` defaults to 1
/// (reference `head_json.contains("head_dilation")` guard).
#[derive(Deserialize)]
struct NewLayerHeadConfig {
    out_channels: usize,
    kernel_size: usize,
    #[serde(default = "default_one")]
    head_dilation: usize,
    bias: bool,
}

#[derive(Deserialize)]
struct NewLayer1x1Config {
    #[serde(default = "default_one")]
    groups: usize,
}

/// All three fields are required when the object is present, matching the
/// reference (`head1x1_config["active"]` / `["out_channels"]` / `["groups"]`
/// in NAM/wavenet/model.cpp throw on absence) and the typed A2 parser.
#[derive(Deserialize)]
struct NewHead1x1Config {
    active: bool,
    out_channels: usize,
    groups: usize,
}

fn default_true() -> bool {
    true
}

fn default_one() -> usize {
    1
}

#[derive(Deserialize)]
struct NewHeadConfig {
    channels: usize,
    num_layers: usize,
    out_channels: usize,
}

#[derive(Deserialize)]
struct NewWaveNetConfig {
    layers: Vec<NewLayerArrayConfig>,
    head: Option<NewHeadConfig>,
    #[serde(default)]
    #[allow(dead_code)]
    head_scale: Option<f32>,
    /// A2 nested condition_dsp model object, kept as raw JSON; the engine
    /// constructs it recursively (`build_condition_dsp`). JSON `null`
    /// deserializes to `None`, matching the reference's
    /// `!config["condition_dsp"].is_null()` guard.
    #[serde(default)]
    condition_dsp: Option<serde_json::Value>,
}

impl NewWaveNetConfig {
    fn into_config(self) -> Result<WaveNetConfig, String> {
        let first = self
            .layers
            .first()
            .ok_or("WaveNet config has no layer arrays")?;
        let first_input_size = first.input_size;

        let stacks: Vec<StackConfig> = self
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let ks = if let Some(ref ks) = l.kernel_sizes {
                    ks.clone()
                } else {
                    let k = l.kernel_size.unwrap_or(2);
                    vec![k; l.dilations.len()]
                };
                // Per-layer activations through the shared A2 parsing:
                // single config broadcast, or an array matching the layer
                // count (e.g. the wavenet_a2_max nested condition_dsp).
                let activations = parse_activation_value(
                    l.activation.as_ref().filter(|v| !v.is_null()),
                    l.dilations.len(),
                    &format!("Layer array {i}"),
                )?;
                // Per-layer gating modes + secondary activations, through
                // the typed A2 parsing (single source of gating semantics).
                let (gating_modes, secondary_activations) = parse_gating_config(
                    l.gating_mode.as_ref(),
                    l.gated,
                    l.secondary_activation.as_ref(),
                    l.dilations.len(),
                    &format!("Layer array {i}"),
                )?;
                // Head rechannel: prefer the nested `head` object (A2 /
                // trainer export); legacy files use flat head_size +
                // head_bias with an implicit kernel of 1 (reference
                // `parse_config_json` precedence).
                let (head_size, head_kernel_size, head_dilation, head_bias) = match &l.head {
                    Some(h) => {
                        if h.kernel_size == 0 {
                            return Err(format!(
                                "Layer array {i}: head.kernel_size must be >= 1"
                            ));
                        }
                        (h.out_channels, h.kernel_size, h.head_dilation, h.bias)
                    }
                    None => match l.head_size {
                        Some(hs) => (hs, 1, 1, l.head_bias),
                        None => {
                            return Err(format!(
                                "Layer array {i}: expected 'head' object with out_channels, kernel_size, and bias, or legacy 'head_size' and 'head_bias'"
                            ));
                        }
                    },
                };
                Ok(StackConfig {
                    input_size: l.input_size,
                    condition_size: l.condition_size,
                    head_size,
                    head_kernel_size,
                    head_dilation,
                    head_bias,
                    channels: l.channels,
                    bottleneck: l.bottleneck.unwrap_or(l.channels),
                    dilations: l.dilations.clone(),
                    kernel_sizes: ks,
                    activations,
                    gating_modes,
                    secondary_activations,
                    groups_input: l.groups_input,
                    groups_input_mixin: l.groups_input_mixin,
                    layer1x1_groups: l.layer1x1.as_ref().map(|c| c.groups).unwrap_or(1),
                    head1x1: l.head1x1.as_ref().map_or(
                        Head1x1Params::inactive(l.channels),
                        |h| Head1x1Params {
                            active: h.active,
                            out_channels: h.out_channels,
                            groups: h.groups,
                        },
                    ),
                    films: l.films(&format!("Layer array {i}"))?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        let (head, head_size) = match self.head {
            Some(h) => {
                let hidden = if h.num_layers > 0 {
                    vec![h.channels; h.num_layers]
                } else {
                    vec![]
                };
                (hidden, h.out_channels)
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
            condition_dsp: self.condition_dsp,
        })
    }
}

// -- Full A2 config surface ---------------------------------------------------

/// Parse a WaveNet config covering the full A2 surface into typed structs
/// (see [`super::wavenet::params`]): activation config objects/arrays,
/// `bottleneck`, blended gating + `secondary_activation`, grouped
/// convolutions, `layer1x1`/`head1x1` objects, the 8 FiLM insertion points,
/// windowed heads, `condition_dsp`, and `slimmable` descriptors. Plain A1
/// layer-array configs parse too, with every A2 field at its A1-equivalent
/// default.
///
/// `load_model_from_file` does not consume the full typed config yet — model
/// construction is still driven by [`parse_wavenet_config`], which shares
/// the typed gating parsing (`parse_gating_config`) so all three gating
/// modes reach inference; the remaining A2 surface (FiLM, groups, head1x1,
/// windowed head, condition_dsp, slimmable) keeps failing at later stages
/// until its inference todos land.
pub fn parse_full_wavenet_config(
    value: &serde_json::Value,
) -> Result<super::wavenet::params::WaveNetFullConfig, String> {
    super::wavenet::params::WaveNetFullConfig::from_json(value)
}

// -- Shared -----------------------------------------------------------------

/// Parse a WaveNet `config` JSON object (old flat or new layer-array format)
/// into the engine's [`WaveNetConfig`]. This is the exact parse
/// `load_model_from_file` uses before model construction; exposed so tests
/// can assert what reaches the engine for a given config. Unknown A2 fields
/// the engine does not consume yet (e.g. FiLM insertion points) are ignored.
pub fn parse_wavenet_config(value: serde_json::Value) -> Result<WaveNetConfig, String> {
    // Try old format first (flat config with integer layer counts).
    if let Ok(old) = serde_json::from_value::<OldWaveNetConfig>(value.clone()) {
        return old.into_config();
    }
    // Try new format (layer array config objects).
    let new_cfg: NewWaveNetConfig =
        serde_json::from_value(value).map_err(|e| format!("Invalid WaveNet config: {e}"))?;
    new_cfg.into_config()
}

// -- condition_dsp sub-network ------------------------------------------------

/// Shape of a nested `condition_dsp` model object: a complete .nam-style
/// model (`architecture`/`config`/`weights`/optional `sample_rate`) embedded
/// under the outer WaveNet config. The reference (`parse_config_json` in
/// NAM/wavenet/model.cpp) builds it eagerly via `nam::get_dsp`, so its
/// weights come from this object's own `weights` array — NOT from the outer
/// flat weight stream, which `WaveNet::set_weights_` consumes for the layer
/// arrays + head only ("condition_dsp already has its own weights from
/// construction"). The outer stream layout is therefore identical with or
/// without a condition_dsp.
#[derive(Deserialize)]
struct NestedModelFile {
    architecture: String,
    config: serde_json::Value,
    weights: Vec<f32>,
}

/// Construct the A2 `condition_dsp` sub-network from its raw nested model
/// JSON, recursively through the engine's normal WaveNet parse/construction
/// (a nested net may itself carry the full A2 surface — bottleneck, gating,
/// FiLM, head1x1, even its own condition_dsp, exactly as the reference's
/// recursive `get_dsp` allows).
///
/// A2 training only emits WaveNet sub-networks; any other nested
/// architecture is rejected with a clear error. The nested weight array must
/// be consumed exactly (the reference `set_weights_` throws on leftovers).
pub(crate) fn build_condition_dsp(value: &serde_json::Value) -> Result<WaveNetModel, String> {
    let nested: NestedModelFile = serde_json::from_value(value.clone())
        .map_err(|e| format!("Invalid condition_dsp model: {e}"))?;
    if nested.architecture != "WaveNet" {
        return Err(format!(
            "Unsupported condition_dsp architecture: {} (only WaveNet condition_dsp sub-networks are supported)",
            nested.architecture
        ));
    }
    let config = parse_wavenet_config(nested.config).map_err(|e| format!("condition_dsp: {e}"))?;
    let mut reader = WeightReader::new(&nested.weights);
    let model = WaveNetModel::from_config_and_weights(config, &mut reader)
        .map_err(|e| format!("condition_dsp: {e}"))?;
    if reader.remaining() > 0 {
        return Err(format!(
            "condition_dsp: {} unused weights after construction",
            reader.remaining()
        ));
    }
    Ok(model)
}

#[derive(Deserialize)]
pub struct LstmConfig {
    pub input_size: usize,
    pub hidden_size: usize,
    pub num_layers: usize,
}

/// Helper to sequentially consume weights from a flat array.
pub struct WeightReader<'a> {
    weights: &'a [f32],
    pos: usize,
}

impl<'a> WeightReader<'a> {
    pub fn new(weights: &'a [f32]) -> Self {
        Self { weights, pos: 0 }
    }

    pub fn read(&mut self, count: usize) -> Result<Vec<f32>, String> {
        if self.pos + count > self.weights.len() {
            return Err(format!(
                "Weight underflow: need {} more but only {} remain (at pos {})",
                count,
                self.weights.len() - self.pos,
                self.pos
            ));
        }
        let slice = self.weights[self.pos..self.pos + count].to_vec();
        self.pos += count;
        Ok(slice)
    }

    pub fn remaining(&self) -> usize {
        self.weights.len() - self.pos
    }
}

// -- Slimmable models ---------------------------------------------------------

/// Raw-JSON probe: does any layer array of a (new-format) WaveNet config
/// carry a non-null `slimmable` descriptor? Old flat configs have no
/// `layers` array and are never slimmable; non-slimmable files must keep
/// taking the historical load path bit-for-bit, so this never rejects — the
/// typed parse validates the descriptors once a file IS slimmable.
fn wavenet_config_is_slimmable(config: &serde_json::Value) -> bool {
    config
        .get("layers")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|layers| {
            layers
                .iter()
                .any(|l| l.get("slimmable").is_some_and(|s| !s.is_null()))
        })
}

/// One `SlimmableContainer` submodel descriptor: the size threshold this
/// submodel covers values up to, and its complete nested .nam-style model
/// object (reference `ContainerConfig::create` in NAM/container.cpp).
#[derive(Deserialize)]
struct ContainerSubmodelEntry {
    max_value: f64,
    model: serde_json::Value,
}

#[derive(Deserialize)]
struct ContainerConfigFile {
    submodels: Vec<ContainerSubmodelEntry>,
}

/// A container submodel's nested model spec. Unlike `condition_dsp` nets it
/// carries its own `sample_rate`, validated against the container's.
#[derive(Deserialize)]
struct SubmodelFile {
    architecture: String,
    config: serde_json::Value,
    weights: Vec<f32>,
    #[serde(default)]
    sample_rate: Option<serde_json::Value>,
}

/// Reference `ContainerModel::_get_index_for_slimmable_size`: the first
/// submodel whose `max_value` exceeds the requested size; the last submodel
/// is the fallback (and the load-time default, size = 1.0 -> full).
fn container_submodel_index(max_values: &[f64], size: f64) -> usize {
    max_values
        .iter()
        .position(|&mv| size < mv)
        .unwrap_or(max_values.len() - 1)
}

// -- Model construction -------------------------------------------------------

/// Construct a model from its parts, dispatching on the architecture.
/// Shared by the top-level file load and `SlimmableContainer` submodels
/// (which recurse through it with their own nested spec).
///
/// `strict_weights` controls the leftover-weight policy: container
/// submodels and slimmable WaveNets must consume their weight vector
/// exactly (the slice/layout verification of todo #1112), while top-level
/// non-slimmable files keep the historical lenient warning so A1 and plain
/// A2 files load bit-for-bit unchanged.
fn build_model(
    architecture: &str,
    config: serde_json::Value,
    weights: &[f32],
    sample_rate: f32,
    strict_weights: bool,
) -> Result<Box<dyn NamInference>, String> {
    let mut reader = WeightReader::new(weights);
    match architecture {
        "WaveNet" => {
            let slimmable = wavenet_config_is_slimmable(&config);
            if slimmable {
                // Typed parse first (before any weights are consumed): it
                // validates every slimmable descriptor — unknown methods,
                // unsorted allowed_channels, last entry != channels — with
                // clear errors (see params::SlimmableParams).
                let full = parse_full_wavenet_config(&config)?;
                if full.head.is_some() {
                    return Err(
                        "Slimmable WaveNet: a post-stack head is not supported".to_string()
                    );
                }
                let target = wavenet::slimmable::channels_for_size(
                    &full.layer_arrays,
                    wavenet::slimmable::FULL_SIZE,
                );
                // v1 always selects the full size, and full allowed_channels
                // lists end at the full channel count (validated), so the
                // slice is the whole packed vector and the weights below
                // flow to construction unchanged. Smaller sizes additionally
                // need `derive_params_for_channels` + engine-config
                // rederivation when runtime A2-Lite selection lands
                // (follow-on todo).
                if !wavenet::slimmable::is_full_size(&full.layer_arrays, &target) {
                    return Err(
                        "Slimmable WaveNet: only the full-size slice is supported (runtime A2-Lite size selection is not implemented yet)"
                            .to_string(),
                    );
                }
                // The full-size extraction walk verifies the packed vector
                // against the full-size layout exactly (every tensor,
                // head_scale included) and returns it bit-identically.
                let slice = wavenet::slimmable::extract_slimmed_weights(
                    &full.layer_arrays,
                    weights,
                    &target,
                )?;
                debug_assert_eq!(slice, weights, "full-size slice must be the identity");
            }
            let config = parse_wavenet_config(config)?;
            // Reference `parse_config_json`: a condition_dsp trained at a
            // different rate than the outer model is a broken export.
            if let Some(cd) = &config.condition_dsp {
                let nested_rate = parse_sample_rate(cd.get("sample_rate"));
                if nested_rate != sample_rate {
                    return Err(format!(
                        "condition_dsp expected sample rate ({nested_rate}) doesn't match model sample rate ({sample_rate})"
                    ));
                }
            }
            let model = WaveNetModel::from_config_and_weights(config, &mut reader)?;
            if reader.remaining() > 0 {
                if slimmable {
                    // The packed vector must BE the full-size layout; a
                    // mismatch means a broken slice descriptor or export.
                    return Err(format!(
                        "Slimmable WaveNet: {} unused weights after loading the full-size slice (packed vector doesn't match the full-size layout)",
                        reader.remaining()
                    ));
                }
                if strict_weights {
                    return Err(format!(
                        "{} unused weights after loading WaveNet model",
                        reader.remaining()
                    ));
                }
                eprintln!(
                    "Warning: {} unused weights after loading WaveNet model",
                    reader.remaining()
                );
            }
            Ok(Box::new(model))
        }
        "LSTM" => {
            let config: LstmConfig = serde_json::from_value(config)
                .map_err(|e| format!("Invalid LSTM config: {e}"))?;
            let model = LstmModel::from_config_and_weights(config, &mut reader)?;
            if reader.remaining() > 0 {
                if strict_weights {
                    return Err(format!(
                        "{} unused weights after loading LSTM model",
                        reader.remaining()
                    ));
                }
                eprintln!(
                    "Warning: {} unused weights after loading LSTM model",
                    reader.remaining()
                );
            }
            Ok(Box::new(model))
        }
        // A2 slimmable container (e.g. the A2.nam trainer export): one file
        // holding complete submodels at different sizes (reference
        // NAM/container.cpp). The top-level weight vector is unused — every
        // submodel carries its own. v1 constructs the FULL (last) submodel
        // only; the descriptor validation and index selection mirror the
        // reference so runtime size selection can pick another submodel
        // later without reshaping this path.
        "SlimmableContainer" => {
            let cfg: ContainerConfigFile = serde_json::from_value(config)
                .map_err(|e| format!("Invalid SlimmableContainer config: {e}"))?;
            if cfg.submodels.is_empty() {
                return Err("SlimmableContainer: 'submodels' must be a non-empty array".to_string());
            }
            for pair in cfg.submodels.windows(2) {
                if pair[1].max_value <= pair[0].max_value {
                    return Err(
                        "SlimmableContainer: submodels must be sorted by ascending max_value"
                            .to_string(),
                    );
                }
            }
            let max_values: Vec<f64> = cfg.submodels.iter().map(|s| s.max_value).collect();
            if *max_values.last().expect("non-empty") < 1.0 {
                return Err(
                    "SlimmableContainer: last submodel max_value must be >= 1.0".to_string()
                );
            }
            let index = container_submodel_index(&max_values, wavenet::slimmable::FULL_SIZE);
            let entry = cfg
                .submodels
                .into_iter()
                .nth(index)
                .expect("index selected from the same list");
            let sub: SubmodelFile = serde_json::from_value(entry.model)
                .map_err(|e| format!("SlimmableContainer: invalid submodel {index}: {e}"))?;
            // Reference ContainerModel ctor: a submodel trained at a
            // different rate than the container is a broken export (an
            // absent submodel rate counts as unknown and skips the check).
            if let Some(v) = &sub.sample_rate {
                let sub_rate = parse_sample_rate(Some(v));
                if sub_rate != sample_rate {
                    return Err(format!(
                        "SlimmableContainer: submodel {index} sample rate ({sub_rate}) doesn't match container sample rate ({sample_rate})"
                    ));
                }
            }
            build_model(&sub.architecture, sub.config, &sub.weights, sample_rate, true)
        }
        other => Err(format!("Unsupported architecture: {other}")),
    }
}

/// Load a NAM model from a .nam file path.
pub fn load_model_from_file(path: &str) -> Result<LoadedModel, String> {
    let data = std::fs::read_to_string(Path::new(path))
        .map_err(|e| format!("Failed to read file: {e}"))?;

    let nam_file: NamFile =
        serde_json::from_str(&data).map_err(|e| format!("Failed to parse JSON: {e}"))?;

    let sample_rate = parse_sample_rate(nam_file.sample_rate.as_ref());
    let model = build_model(
        &nam_file.architecture,
        nam_file.config,
        &nam_file.weights,
        sample_rate,
        false,
    )?;

    Ok(LoadedModel { model, sample_rate })
}
