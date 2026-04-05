/// NAM .nam file parsing and model construction.

use serde::Deserialize;
use std::path::Path;

fn null_as_empty_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<Vec<T>>::deserialize(deserializer).map(|opt| opt.unwrap_or_default())
}

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

#[derive(Deserialize)]
pub struct WaveNetConfig {
    pub input_size: usize,
    pub condition_size: usize,
    pub head_size: usize,
    pub channels: usize,
    /// Number of layers per stack, e.g. [10, 10] = 2 stacks of 10 layers.
    #[serde(deserialize_with = "null_as_empty_vec")]
    pub layers: Vec<usize>,
    /// Head hidden layer sizes, e.g. [8].
    #[serde(deserialize_with = "null_as_empty_vec")]
    pub head: Vec<usize>,
    pub activation: String,
    pub gated: bool,
    pub head_bias: bool,
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
                "Weight underflow: need {} more but only {} remain",
                count,
                self.weights.len() - self.pos
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
            let config: WaveNetConfig = serde_json::from_value(nam_file.config)
                .map_err(|e| format!("Invalid WaveNet config: {e}"))?;
            let model = WaveNetModel::from_config_and_weights(config, &mut reader)?;
            if reader.remaining() > 0 {
                nih_plug::nih_log!(
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
                nih_plug::nih_log!(
                    "Warning: {} unused weights after loading LSTM model",
                    reader.remaining()
                );
            }
            Ok(Box::new(model))
        }
        other => Err(format!("Unsupported architecture: {other}")),
    }
}
