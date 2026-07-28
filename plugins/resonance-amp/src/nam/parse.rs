/// NAM .nam file parsing and model construction.
use serde::Deserialize;
use std::path::Path;

use super::activations::{ActivationConfig, ActivationKind};
use super::lstm::LstmModel;
use super::wavenet::params::{parse_gating_config, GatingMode};
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
    pub head_bias: bool,
    /// Whether layers have a learned 1x1 residual conv (_layer1x1).
    /// True for new-format NAM models (default), false for old format.
    pub has_layer1x1: bool,
}

pub struct StackConfig {
    pub input_size: usize,
    pub condition_size: usize,
    pub head_size: usize,
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
                    channels: self.channels,
                    // Old-format models predate the A2 bottleneck.
                    bottleneck: self.channels,
                    kernel_sizes: vec![2; n],
                    dilations: d,
                    activation: activation.clone(),
                    gating_modes,
                    secondary_activations,
                }
            })
            .collect();
        Ok(WaveNetConfig {
            input_size: self.input_size,
            stacks,
            head: self.head,
            head_size: self.head_size,
            head_bias: self.head_bias,
            has_layer1x1: false,
        })
    }
}

// -- New NAM format (explicit layer array configs) ---------------------------

#[derive(Deserialize)]
struct NewLayerArrayConfig {
    input_size: usize,
    condition_size: usize,
    head_size: usize,
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
}

fn default_true() -> bool {
    true
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
                Ok(StackConfig {
                    input_size: l.input_size,
                    condition_size: l.condition_size,
                    head_size: l.head_size,
                    channels: l.channels,
                    bottleneck: l.bottleneck.unwrap_or(l.channels),
                    dilations: l.dilations.clone(),
                    kernel_sizes: ks,
                    activation,
                    gating_modes,
                    secondary_activations,
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
            None => (vec![], first.head_size),
        };

        Ok(WaveNetConfig {
            input_size: first.input_size,
            stacks,
            head,
            head_size,
            head_bias: first.head_bias,
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

fn parse_wavenet_config(value: serde_json::Value) -> Result<WaveNetConfig, String> {
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
