//! Reading a `.nam` file's metadata without building (or even allocating)
//! the model: the top-level fields are deserialised and `weights` /
//! `config` are skipped with `serde::de::IgnoredAny`, so a 50 MB model
//! costs one streaming JSON scan and no weight vectors.

use std::io::Read;
use std::path::Path;

use serde::de::IgnoredAny;
use serde::Deserialize;

/// Sample rate assumed when a file omits `sample_rate` (older exporters);
/// the NAM convention, and the amp parser's `DEFAULT_SAMPLE_RATE`.
pub const DEFAULT_SAMPLE_RATE: f64 = 48_000.0;

/// What a `.nam` file says about itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NamHeader {
    /// Exporter version, e.g. `"0.5.4"`.
    pub version: Option<String>,
    /// Top-level `architecture`: `WaveNet`, `LSTM`, `SlimmableContainer`, …
    pub architecture: String,
    /// `sample_rate`, or [`DEFAULT_SAMPLE_RATE`] when absent/unreadable.
    pub sample_rate: f64,
    /// Whether `sample_rate` was actually present.
    pub sample_rate_declared: bool,
    pub name: Option<String>,
    pub modeled_by: Option<String>,
    pub gear_type: Option<String>,
    pub gear_make: Option<String>,
    pub gear_model: Option<String>,
    pub tone_type: Option<String>,
    pub input_level_dbu: Option<f64>,
    pub output_level_dbu: Option<f64>,
    pub loudness: Option<f64>,
    pub gain: Option<f64>,
    /// `metadata.training.validation_esr`.
    pub validation_esr: Option<f64>,
}

impl NamHeader {
    /// A display label for the architecture: `WaveNet A1`, `WaveNet A2`,
    /// `A2 slimmable`, `LSTM`, or the raw name.
    ///
    /// A1 vs A2 is read from the exporter version (0.6 and later write the
    /// A2 layout), not from the config: telling them apart by config shape
    /// would mean parsing `config`, which a slimmable container fills with
    /// nested models' weights.
    pub fn architecture_label(&self) -> String {
        match self.architecture.as_str() {
            "WaveNet" => {
                if self.is_a2_version() {
                    "WaveNet A2".to_string()
                } else {
                    "WaveNet A1".to_string()
                }
            }
            "SlimmableContainer" => "A2 slimmable".to_string(),
            other => other.to_string(),
        }
    }

    fn is_a2_version(&self) -> bool {
        let Some(v) = self.version.as_deref() else {
            return false;
        };
        let mut parts = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
        let major = parts.next().unwrap_or(0);
        let minor = parts.next().unwrap_or(0);
        (major, minor) >= (0, 6)
    }

    /// `gear_make` + `gear_model`, e.g. "Darkglass Electronics Microtubes 900 v2".
    pub fn gear(&self) -> Option<String> {
        match (self.gear_make.as_deref(), self.gear_model.as_deref()) {
            (Some(a), Some(b)) => Some(format!("{a} {b}")),
            (Some(a), None) | (None, Some(a)) => Some(a.to_string()),
            (None, None) => None,
        }
    }
}

/// Why a header could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HeaderError {
    #[error("Failed to read file: {0}")]
    Io(String),
    #[error("not a NAM model: {0}")]
    Parse(String),
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    version: Option<serde_json::Value>,
    architecture: String,
    #[serde(default)]
    sample_rate: Option<serde_json::Value>,
    #[serde(default)]
    metadata: Option<RawMetadata>,
    #[serde(default)]
    #[allow(dead_code)]
    config: Option<IgnoredAny>,
    #[allow(dead_code)]
    weights: IgnoredAny,
}

#[derive(Deserialize, Default)]
struct RawMetadata {
    #[serde(default)]
    name: Option<serde_json::Value>,
    #[serde(default)]
    modeled_by: Option<serde_json::Value>,
    #[serde(default)]
    gear_type: Option<serde_json::Value>,
    #[serde(default)]
    gear_make: Option<serde_json::Value>,
    #[serde(default)]
    gear_model: Option<serde_json::Value>,
    #[serde(default)]
    tone_type: Option<serde_json::Value>,
    #[serde(default)]
    input_level_dbu: Option<serde_json::Value>,
    #[serde(default)]
    output_level_dbu: Option<serde_json::Value>,
    #[serde(default)]
    loudness: Option<serde_json::Value>,
    #[serde(default)]
    gain: Option<serde_json::Value>,
    #[serde(default)]
    training: Option<RawTraining>,
}

#[derive(Deserialize, Default)]
struct RawTraining {
    #[serde(default)]
    validation_esr: Option<serde_json::Value>,
}

/// A non-empty string field (numbers are stringified; exporters vary).
fn text(v: Option<serde_json::Value>) -> Option<String> {
    match v? {
        serde_json::Value::String(s) => {
            let s = s.trim().to_string();
            (!s.is_empty()).then_some(s)
        }
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A finite number field. `"inf"`, `"NaN"` and friends read as absent: a
/// non-finite value would serialise as `null` in the index and could never
/// be read back.
fn number(v: Option<serde_json::Value>) -> Option<f64> {
    let n = match v? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }?;
    n.is_finite().then_some(n)
}

/// Parse a header from any reader (buffer it: the scan is byte-wise).
pub fn read_header_from<R: Read>(reader: R) -> Result<NamHeader, HeaderError> {
    let raw: RawFile =
        serde_json::from_reader(reader).map_err(|e| HeaderError::Parse(e.to_string()))?;
    let declared = number(raw.sample_rate.clone()).filter(|r| *r > 0.0);
    let meta = raw.metadata.unwrap_or_default();
    Ok(NamHeader {
        version: text(raw.version),
        architecture: raw.architecture,
        sample_rate: declared.unwrap_or(DEFAULT_SAMPLE_RATE),
        sample_rate_declared: declared.is_some(),
        name: text(meta.name),
        modeled_by: text(meta.modeled_by),
        gear_type: text(meta.gear_type),
        gear_make: text(meta.gear_make),
        gear_model: text(meta.gear_model),
        tone_type: text(meta.tone_type),
        input_level_dbu: number(meta.input_level_dbu),
        output_level_dbu: number(meta.output_level_dbu),
        loudness: number(meta.loudness),
        gain: number(meta.gain),
        validation_esr: meta.training.and_then(|t| number(t.validation_esr)),
    })
}

/// Read a `.nam` file's header. Never builds the model.
pub fn read_header(path: &Path) -> Result<NamHeader, HeaderError> {
    let file = std::fs::File::open(path).map_err(|e| HeaderError::Io(e.to_string()))?;
    read_header_from(std::io::BufReader::with_capacity(64 * 1024, file))
}
