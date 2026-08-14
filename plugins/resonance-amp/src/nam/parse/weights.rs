//! Weight-stream consumption and model construction.
//!
//! Everything here works on an already-parsed config: [`WeightReader`] hands
//! out slices of the flat weight array in reference order, and
//! [`build_model`] dispatches on the architecture string. Config *meaning*
//! lives in [`super::config`]; container submodel selection lives in
//! [`super::container`].

use super::super::lstm::LstmModel;
use super::super::wavenet::WaveNetModel;
use super::super::NamInference;
use super::config::{check_condition_dsp_rates, parse_wavenet_config};
use super::schema::{LstmConfig, NestedModelFile};
use super::{container, engine_config::WaveNetConfig};

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

/// Construct a model from its parts, dispatching on the architecture.
/// Shared by the top-level file load and `SlimmableContainer` submodels
/// (which recurse through it with their own nested spec).
///
/// `strict_weights` controls the leftover-weight policy: container
/// submodels and slimmable WaveNets must consume their weight vector
/// exactly (the slice/layout verification of todo #1112), while top-level
/// non-slimmable files keep the historical lenient warning so A1 and plain
/// A2 files load bit-for-bit unchanged.
///
/// `force_exact_activations` forces the exact (A2) activation flavor
/// regardless of the config's own A2 markers: `SlimmableContainer`
/// submodels only exist in the A2 era, so even an A1-shaped submodel runs
/// with the exact activations, exactly as the reference `nam::get_dsp`
/// (which never enables fast tanh for fixture renders) would.
pub(super) fn build_model(
    architecture: &str,
    config: serde_json::Value,
    weights: &[f32],
    sample_rate: f32,
    strict_weights: bool,
    force_exact_activations: bool,
) -> Result<Box<dyn NamInference>, String> {
    match architecture {
        "WaveNet" => build_wavenet(
            config,
            weights,
            sample_rate,
            strict_weights,
            force_exact_activations,
        ),
        "LSTM" => build_lstm(config, weights, strict_weights),
        "SlimmableContainer" => container::build_container(config, sample_rate),
        other => Err(format!("Unsupported architecture: {other}")),
    }
}

fn build_wavenet(
    config: serde_json::Value,
    weights: &[f32],
    sample_rate: f32,
    strict_weights: bool,
    force_exact_activations: bool,
) -> Result<Box<dyn NamInference>, String> {
    let slimmable = container::wavenet_config_is_slimmable(&config);
    if slimmable {
        container::verify_full_size_slice(&config, weights)?;
    }
    // Reference `parse_config_json`: a condition_dsp trained at a
    // different rate than the outer model is a broken export. The
    // reference checks this at EVERY nesting level (each recursive
    // `get_dsp` re-validates its own condition_dsp), so walk the
    // raw JSON down through nested condition_dsp models too.
    check_condition_dsp_rates(&config, sample_rate)?;
    let mut config = parse_wavenet_config(config)?;
    config.fast_activations &= !force_exact_activations;

    let mut reader = WeightReader::new(weights);
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

fn build_lstm(
    config: serde_json::Value,
    weights: &[f32],
    strict_weights: bool,
) -> Result<Box<dyn NamInference>, String> {
    let config: LstmConfig =
        serde_json::from_value(config).map_err(|e| format!("Invalid LSTM config: {e}"))?;
    let mut reader = WeightReader::new(weights);
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
    let mut config: WaveNetConfig =
        parse_wavenet_config(nested.config).map_err(|e| format!("condition_dsp: {e}"))?;
    // A condition_dsp only exists in the A2 era, and the reference builds
    // it through the same `get_dsp` semantics as the outer net — even when
    // the nested config itself looks A1-shaped (e.g. the
    // wavenet_condition_dsp fixture's plain-Tanh sub-network), it must run
    // with the exact activations its reference renders used.
    config.fast_activations = false;
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
