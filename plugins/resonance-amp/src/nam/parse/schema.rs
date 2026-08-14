//! Serde mirrors of the on-disk `.nam` shapes.
//!
//! Everything in here is a faithful mirror of a JSON envelope — the
//! top-level file, the nested models a config can embed, and the two config
//! formats that are NOT covered by the typed A2 surface
//! ([`crate::nam::wavenet::params::WaveNetFullConfig`]): the flat A1 WaveNet
//! config and the LSTM config. Translation of these into engine configs
//! lives in [`super::config`]; weight consumption lives in
//! [`super::weights`].

use serde::Deserialize;

use super::DEFAULT_SAMPLE_RATE;

/// The top-level `.nam` file envelope.
#[derive(Deserialize)]
pub(super) struct NamFile {
    #[allow(dead_code)]
    pub(super) version: Option<String>,
    pub(super) architecture: String,
    pub(super) config: serde_json::Value,
    pub(super) weights: Vec<f32>,
    /// Rate the profile was captured/trained at. Kept as a raw JSON value
    /// because exporters write it as int, float, or (rarely) a string.
    #[serde(default)]
    pub(super) sample_rate: Option<serde_json::Value>,
}

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
pub(super) struct NestedModelFile {
    pub(super) architecture: String,
    pub(super) config: serde_json::Value,
    pub(super) weights: Vec<f32>,
}

/// One `SlimmableContainer` submodel descriptor: the size threshold this
/// submodel covers values up to, and its complete nested .nam-style model
/// object (reference `ContainerConfig::create` in NAM/container.cpp).
#[derive(Deserialize)]
pub(super) struct ContainerSubmodelEntry {
    pub(super) max_value: f64,
    pub(super) model: serde_json::Value,
}

#[derive(Deserialize)]
pub(super) struct ContainerConfigFile {
    pub(super) submodels: Vec<ContainerSubmodelEntry>,
}

/// A container submodel's nested model spec. Unlike `condition_dsp` nets it
/// carries its own `sample_rate`, validated against the container's.
#[derive(Deserialize)]
pub(super) struct SubmodelFile {
    pub(super) architecture: String,
    pub(super) config: serde_json::Value,
    pub(super) weights: Vec<f32>,
    #[serde(default)]
    pub(super) sample_rate: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct LstmConfig {
    pub input_size: usize,
    pub hidden_size: usize,
    pub num_layers: usize,
}

/// The original flat WaveNet config: layer *counts* instead of layer-array
/// objects, one activation for the whole model, and a boolean `gated`. It is
/// the one WaveNet config shape the typed A2 surface deliberately does not
/// model (that surface requires a `layers` array of objects), so it keeps its
/// own serde mirror; [`super::config`] translates it. Its A1-equivalent
/// defaults for every later field are spelled out there.
#[derive(Deserialize)]
pub(super) struct OldWaveNetConfig {
    pub(super) input_size: usize,
    pub(super) condition_size: usize,
    pub(super) head_size: usize,
    pub(super) channels: usize,
    pub(super) layers: Vec<usize>,
    pub(super) head: Vec<usize>,
    pub(super) activation: String,
    pub(super) gated: bool,
    pub(super) head_bias: bool,
}

/// Coerce a `sample_rate` JSON value (int, float, or string) to a usable
/// rate, falling back to [`DEFAULT_SAMPLE_RATE`] when absent or unusable.
pub(super) fn parse_sample_rate(value: Option<&serde_json::Value>) -> f32 {
    value
        .and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        })
        .map(|r| r as f32)
        .filter(|r| *r > 0.0)
        .unwrap_or(DEFAULT_SAMPLE_RATE)
}
