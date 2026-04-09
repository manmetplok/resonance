/// NAM .nam file parsing and model construction.

use serde::Deserialize;
use std::path::Path;

use super::lstm::LstmModel;
use super::wavenet::WaveNetModel;
use super::NamInference;

#[derive(Deserialize)]
struct NamFile {
    #[allow(dead_code)]
    version: Option<String>,
    architecture: String,
    config: serde_json::Value,
    weights: Vec<f32>,
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
    pub gated: bool,
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
    pub dilations: Vec<usize>,
    pub kernel_sizes: Vec<usize>,
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
    #[allow(dead_code)]
    activation: String,
    gated: bool,
    head_bias: bool,
}

impl OldWaveNetConfig {
    fn into_config(self) -> WaveNetConfig {
        let dilations: Vec<Vec<usize>> = self
            .layers
            .iter()
            .map(|&n| (0..n).map(|i| 1usize << i).collect())
            .collect();
        let stacks = dilations
            .into_iter()
            .map(|d| {
                let n = d.len();
                StackConfig {
                    input_size: self.input_size,
                    condition_size: self.condition_size,
                    head_size: self.head_size,
                    channels: self.channels,
                    kernel_sizes: vec![2; n],
                    dilations: d,
                }
            })
            .collect();
        WaveNetConfig {
            input_size: self.input_size,
            stacks,
            head: self.head,
            head_size: self.head_size,
            gated: self.gated,
            head_bias: self.head_bias,
            has_layer1x1: false,
        }
    }
}

// -- New NAM format (explicit layer array configs) ---------------------------

#[derive(Deserialize)]
struct NewLayerArrayConfig {
    input_size: usize,
    condition_size: usize,
    head_size: usize,
    channels: usize,
    dilations: Vec<usize>,
    #[serde(default)]
    kernel_size: Option<usize>,
    #[serde(default)]
    kernel_sizes: Option<Vec<usize>>,
    #[serde(default)]
    gated: Option<bool>,
    #[serde(default)]
    gating_mode: Option<serde_json::Value>,
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

        // Determine gating mode.
        let gated = determine_gated(first)?;
        for (i, layer) in self.layers.iter().enumerate().skip(1) {
            if determine_gated(layer)? != gated {
                return Err(format!(
                    "Layer array {} has different gating than layer array 0 (mixed gating not supported)",
                    i
                ));
            }
        }

        let stacks: Vec<StackConfig> = self
            .layers
            .iter()
            .map(|l| {
                let ks = if let Some(ref ks) = l.kernel_sizes {
                    ks.clone()
                } else {
                    let k = l.kernel_size.unwrap_or(2);
                    vec![k; l.dilations.len()]
                };
                StackConfig {
                    input_size: l.input_size,
                    condition_size: l.condition_size,
                    head_size: l.head_size,
                    channels: l.channels,
                    dilations: l.dilations.clone(),
                    kernel_sizes: ks,
                }
            })
            .collect();

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
            gated,
            head_bias: first.head_bias,
            has_layer1x1: true,
        })
    }
}

/// Extract the boolean gated flag from a layer array config.
fn determine_gated(layer: &NewLayerArrayConfig) -> Result<bool, String> {
    if let Some(ref gm) = layer.gating_mode {
        match gm {
            serde_json::Value::String(s) => match s.as_str() {
                "none" => Ok(false),
                "gated" => Ok(true),
                other => Err(format!("Unsupported gating_mode: {other}")),
            },
            serde_json::Value::Array(arr) => {
                let first = arr
                    .first()
                    .and_then(|v| v.as_str())
                    .unwrap_or("none");
                if first != "gated" && first != "none" {
                    return Err(format!("Unsupported gating_mode: {first}"));
                }
                for v in arr {
                    if v.as_str().unwrap_or("none") != first {
                        return Err(
                            "Mixed per-layer gating modes not supported".into(),
                        );
                    }
                }
                Ok(first == "gated")
            }
            _ => Ok(false),
        }
    } else {
        Ok(layer.gated.unwrap_or(false))
    }
}

// -- Shared -----------------------------------------------------------------

fn parse_wavenet_config(value: serde_json::Value) -> Result<WaveNetConfig, String> {
    // Try old format first (flat config with integer layer counts).
    if let Ok(old) = serde_json::from_value::<OldWaveNetConfig>(value.clone()) {
        return Ok(old.into_config());
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
pub fn load_model_from_file(path: &str) -> Result<Box<dyn NamInference>, String> {
    let data = std::fs::read_to_string(Path::new(path))
        .map_err(|e| format!("Failed to read file: {e}"))?;

    let nam_file: NamFile =
        serde_json::from_str(&data).map_err(|e| format!("Failed to parse JSON: {e}"))?;

    let mut reader = WeightReader::new(&nam_file.weights);

    match nam_file.architecture.as_str() {
        "WaveNet" => {
            let config = parse_wavenet_config(nam_file.config)?;
            let model = WaveNetModel::from_config_and_weights(config, &mut reader)?;
            if reader.remaining() > 0 {
                eprintln!(
                    "Warning: {} unused weights after loading WaveNet model",
                    reader.remaining()
                );
            }
            Ok(Box::new(model))
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
            Ok(Box::new(model))
        }
        other => Err(format!("Unsupported architecture: {other}")),
    }
}
