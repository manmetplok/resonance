//! NAM `.nam` file parsing and model construction.
//!
//! The pipeline is split by job, one module per stage:
//!
//! * [`schema`] — serde mirrors of the on-disk envelopes (top-level file,
//!   nested `condition_dsp` model, container submodels, LSTM config) plus
//!   the flat A1 WaveNet config, and sample-rate coercion.
//! * [`engine_config`] — the [`WaveNetConfig`] / [`StackConfig`] structs the
//!   inference engine consumes.
//! * [`config`] — translation of a raw `config` JSON object into that engine
//!   config, and the A2-marker probe that picks the activation flavor.
//! * [`weights`] — the flat weight stream reader and per-architecture model
//!   construction.
//! * [`container`] — `SlimmableContainer` submodel selection and the
//!   slimmable full-size slice verification.
//!
//! This file owns only the public entry point, [`load_model_from_file`].
//!
//! # One config parser
//!
//! Every WaveNet `config` object in the new (layer-array) format is read by
//! exactly one parser: [`super::wavenet::params::WaveNetFullConfig`], the
//! typed A2 surface. [`parse_wavenet_config`] translates that typed config
//! into the engine's [`WaveNetConfig`] — it does not re-read the JSON. A key
//! honoured by the typed surface therefore cannot be silently dropped on the
//! way to inference. The only config JSON read anywhere else is:
//!
//! * the flat A1 format (`schema::OldWaveNetConfig`), which the typed surface
//!   deliberately does not model — it has no `layers` array of objects;
//! * the raw-JSON probes [`config_has_a2_markers`] (activation flavor) and
//!   `container::wavenet_config_is_slimmable` (packed-weight layout), which
//!   only ask *whether a key is present*, never what it means;
//! * `condition_dsp` sample rates, walked as raw JSON before construction.

mod config;
mod container;
mod engine_config;
mod schema;
mod weights;

use std::path::Path;

use super::NamInference;

pub use config::{config_has_a2_markers, parse_full_wavenet_config, parse_wavenet_config};
pub use engine_config::{StackConfig, WaveNetConfig};
pub use schema::LstmConfig;
pub use weights::WeightReader;

pub(crate) use weights::{build_condition_dsp, checked_count};

/// Sample rate assumed when a .nam file omits the `sample_rate` field.
/// Older NAM exporters didn't write it; the NAM convention is that such
/// profiles were captured at 48 kHz.
pub const DEFAULT_SAMPLE_RATE: f32 = 48_000.0;

/// A parsed NAM model plus the sample rate it expects to run at.
pub struct LoadedModel {
    pub model: Box<dyn NamInference>,
    pub sample_rate: f32,
}

/// Load a NAM model from a .nam file path.
pub fn load_model_from_file(path: &str) -> Result<LoadedModel, String> {
    let data = std::fs::read_to_string(Path::new(path))
        .map_err(|e| format!("Failed to read file: {e}"))?;

    let nam_file: schema::NamFile =
        serde_json::from_str(&data).map_err(|e| format!("Failed to parse JSON: {e}"))?;

    let sample_rate = schema::parse_sample_rate(nam_file.sample_rate.as_ref());
    let model = weights::build_model(
        &nam_file.architecture,
        nam_file.config,
        &nam_file.weights,
        sample_rate,
        false,
        false,
    )?;

    Ok(LoadedModel { model, sample_rate })
}
