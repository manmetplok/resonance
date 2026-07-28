/// NAM .nam file parsing and model construction.
use serde::Deserialize;
use std::path::Path;

use super::activations::{ActivationConfig, ActivationKind};
use super::lstm::LstmModel;
use super::wavenet::params::{
    parse_gating_config, FilmParams, GatingMode, Head1x1Params, LayerFilms,
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
    /// Layer activation for this stack. A1 models use `"Tanh"`, which
    /// resolves to the fast-tanh path at model construction.
    pub activation: ActivationConfig,
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
            .map(|d| {
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
                    activation: activation.clone(),
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
    /// Activation: plain string (A1) or A2-style config object. Absent in
    /// some old exports; defaults to `"Tanh"`.
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
}

impl NewWaveNetConfig {
    fn into_config(self) -> Result<WaveNetConfig, String> {
        let first = self
            .layers
            .first()
            .ok_or("WaveNet config has no layer arrays")?;

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
                let activation = match l.activation {
                    Some(ref v) => ActivationConfig::from_json(v)?,
                    None => ActivationConfig::simple(ActivationKind::Tanh),
                };
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
                    activation,
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
            input_size: first.input_size,
            stacks,
            head,
            head_size,
            has_layer1x1: true,
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

/// Load a NAM model from a .nam file path.
pub fn load_model_from_file(path: &str) -> Result<LoadedModel, String> {
    let data = std::fs::read_to_string(Path::new(path))
        .map_err(|e| format!("Failed to read file: {e}"))?;

    let nam_file: NamFile =
        serde_json::from_str(&data).map_err(|e| format!("Failed to parse JSON: {e}"))?;

    let sample_rate = parse_sample_rate(nam_file.sample_rate.as_ref());
    let mut reader = WeightReader::new(&nam_file.weights);

    let model: Box<dyn NamInference> = match nam_file.architecture.as_str() {
        "WaveNet" => {
            let config = parse_wavenet_config(nam_file.config)?;
            let model = WaveNetModel::from_config_and_weights(config, &mut reader)?;
            if reader.remaining() > 0 {
                eprintln!(
                    "Warning: {} unused weights after loading WaveNet model",
                    reader.remaining()
                );
            }
            Box::new(model)
        }
        "LSTM" => {
            let config: LstmConfig = serde_json::from_value(nam_file.config)
                .map_err(|e| format!("Invalid LSTM config: {e}"))?;
            let model = LstmModel::from_config_and_weights(config, &mut reader)?;
            if reader.remaining() > 0 {
                eprintln!(
                    "Warning: {} unused weights after loading LSTM model",
                    reader.remaining()
                );
            }
            Box::new(model)
        }
        other => return Err(format!("Unsupported architecture: {other}")),
    };

    Ok(LoadedModel { model, sample_rate })
}
