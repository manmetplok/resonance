use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

/// Acoustic-side `dsconfig.yaml`. Maps the keys the openvpi/Jobsecond and OpenUtau loaders
/// recognise. Everything is optional because voicebanks omit fields they don't use.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DsAcousticConfigRaw {
    pub phonemes: Option<String>,
    pub acoustic: Option<String>,
    pub vocoder: Option<String>,

    pub speakers: Vec<String>,

    pub hidden_size: Option<i32>,
    pub hop_size: Option<i32>,
    pub sample_rate: Option<i32>,
    /// Older voicebanks express this as an integer step count (e.g. 1000); newer voicebanks
    /// with `use_variable_depth` express it as a fractional 0–1 value (e.g. 0.6). Accept both.
    pub max_depth: Option<f32>,

    pub use_key_shift_embed: Option<bool>,
    pub use_speed_embed: Option<bool>,
    pub use_energy_embed: Option<bool>,
    pub use_breathiness_embed: Option<bool>,
    pub use_voicing_embed: Option<bool>,
    pub use_tension_embed: Option<bool>,
    pub use_shallow_diffusion: Option<bool>,
    pub use_continuous_acoustic_embed: Option<bool>,
    pub use_lang_id: Option<bool>,
    pub use_variable_depth: Option<bool>,

    pub predict_dur: Option<bool>,
    pub predict_pitch: Option<bool>,
    pub predict_energy: Option<bool>,
    pub predict_breathiness: Option<bool>,
    pub predict_voicing: Option<bool>,
    pub predict_tension: Option<bool>,

    pub linguistic: Option<String>,
    pub dur: Option<String>,
    pub pitch: Option<String>,
    pub variance: Option<String>,

    pub augmentation_args: Option<AugmentationArgsRaw>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AugmentationArgsRaw {
    pub random_pitch_shifting: Option<AugmentationLeafRaw>,
    pub random_time_stretching: Option<AugmentationLeafRaw>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AugmentationLeafRaw {
    pub range: Option<Vec<f32>>,
    pub scale: Option<f32>,
    pub domain: Option<String>,
}

/// Vocoder `vocoder.yaml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DsVocoderConfigRaw {
    pub name: Option<String>,
    pub model: Option<String>,
    pub num_mel_bins: Option<i32>,
    pub hop_size: Option<i32>,
    pub sample_rate: Option<i32>,
}

/// Resolved acoustic config with absolute paths and defaults applied.
///
/// Many of these fields mirror the on-disk schema for future use; only a
/// subset is read by the current pipeline.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AcousticConfig {
    pub phonemes_path: PathBuf,
    pub acoustic_model: PathBuf,
    pub vocoder_name: String,
    pub speakers: Vec<String>,
    pub speaker_dir: PathBuf,

    pub hidden_size: i32,
    pub hop_size: i32,
    pub sample_rate: i32,
    pub max_depth: f32,

    pub use_energy_embed: bool,
    pub use_breathiness_embed: bool,
    pub use_voicing_embed: bool,
    pub use_tension_embed: bool,
    pub use_shallow_diffusion: bool,

    pub predict_dur: bool,
    pub predict_pitch: bool,
    pub predict_energy: bool,
    pub predict_breathiness: bool,
    pub predict_voicing: bool,
    pub predict_tension: bool,

    pub linguistic_model: Option<PathBuf>,
    pub dur_model: Option<PathBuf>,
    pub pitch_model: Option<PathBuf>,
    pub variance_model: Option<PathBuf>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct VocoderConfig {
    pub name: String,
    pub model_path: PathBuf,
    pub num_mel_bins: i32,
    pub hop_size: i32,
    pub sample_rate: i32,
}

pub fn load_acoustic(path: &Path) -> Result<AcousticConfig> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading acoustic config at {}", path.display()))?;
    let raw: DsAcousticConfigRaw = serde_yml::from_str(&text)
        .with_context(|| format!("parsing acoustic YAML at {}", path.display()))?;

    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let phonemes_rel = raw
        .phonemes
        .clone()
        .context("acoustic config missing required `phonemes` key")?;
    let acoustic_rel = raw
        .acoustic
        .clone()
        .context("acoustic config missing required `acoustic` key")?;

    Ok(AcousticConfig {
        phonemes_path: dir.join(&phonemes_rel),
        acoustic_model: dir.join(&acoustic_rel),
        vocoder_name: raw.vocoder.clone().unwrap_or_default(),
        speakers: raw.speakers.clone(),
        speaker_dir: dir.clone(),
        hidden_size: raw.hidden_size.unwrap_or(256),
        hop_size: raw.hop_size.unwrap_or(512),
        sample_rate: raw.sample_rate.unwrap_or(44100),
        max_depth: raw.max_depth.unwrap_or(-1.0),
        use_energy_embed: raw.use_energy_embed.unwrap_or(false),
        use_breathiness_embed: raw.use_breathiness_embed.unwrap_or(false),
        use_voicing_embed: raw.use_voicing_embed.unwrap_or(false),
        use_tension_embed: raw.use_tension_embed.unwrap_or(false),
        use_shallow_diffusion: raw.use_shallow_diffusion.unwrap_or(false),
        predict_dur: raw.predict_dur.unwrap_or(false),
        predict_pitch: raw.predict_pitch.unwrap_or(false),
        predict_energy: raw.predict_energy.unwrap_or(false),
        predict_breathiness: raw.predict_breathiness.unwrap_or(false),
        predict_voicing: raw.predict_voicing.unwrap_or(false),
        predict_tension: raw.predict_tension.unwrap_or(false),
        linguistic_model: raw.linguistic.map(|p| dir.join(p)),
        dur_model: raw.dur.map(|p| dir.join(p)),
        pitch_model: raw.pitch.map(|p| dir.join(p)),
        variance_model: raw.variance.map(|p| dir.join(p)),
    })
}

pub fn load_vocoder(path: &Path) -> Result<VocoderConfig> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading vocoder config at {}", path.display()))?;
    let raw: DsVocoderConfigRaw = serde_yml::from_str(&text)
        .with_context(|| format!("parsing vocoder YAML at {}", path.display()))?;

    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let model_rel = raw
        .model
        .clone()
        .context("vocoder config missing required `model` key")?;

    Ok(VocoderConfig {
        name: raw.name.unwrap_or_else(|| "vocoder".into()),
        model_path: dir.join(&model_rel),
        num_mel_bins: raw.num_mel_bins.unwrap_or(128),
        hop_size: raw.hop_size.unwrap_or(512),
        sample_rate: raw.sample_rate.unwrap_or(44100),
    })
}

/// Read a phoneme dictionary into a `name -> token-id` map. Auto-detects
/// the on-disk format from the file extension:
/// - `*.txt`  — one phoneme per line, line index = token id (TIGER /
///   older OpenVPI exports)
/// - `*.json` — top-level array of strings *or* object whose keys are
///   phoneme names with explicit integer ids in the values
///   (newer voicebanks like LIEE Lilia and Gahata Meiji)
pub fn load_phoneme_dict(path: &Path) -> Result<std::collections::HashMap<String, i64>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading phoneme dict at {}", path.display()))?;
    if path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
    {
        return load_phoneme_dict_json(&text)
            .with_context(|| format!("parsing JSON phoneme dict at {}", path.display()));
    }
    let mut map = std::collections::HashMap::new();
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim_end_matches('\r').trim();
        if trimmed.is_empty() {
            continue;
        }
        map.insert(trimmed.to_string(), idx as i64);
    }
    Ok(map)
}

/// Parse a JSON phoneme dict. Supports both shapes seen in the wild:
///   - `["AP", "SP", "a", "ae", ...]`   — index = token id
///   - `{"AP": 0, "SP": 1, "a": 2, ...}` — explicit id per phoneme
fn load_phoneme_dict_json(text: &str) -> Result<std::collections::HashMap<String, i64>> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    let mut map = std::collections::HashMap::new();
    match value {
        serde_json::Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                let name = item
                    .as_str()
                    .ok_or_else(|| anyhow!("phoneme dict array entry {idx} is not a string"))?;
                map.insert(name.to_string(), idx as i64);
            }
        }
        serde_json::Value::Object(entries) => {
            for (name, id_value) in entries {
                let id = id_value
                    .as_i64()
                    .ok_or_else(|| anyhow!("phoneme `{name}` has non-integer id"))?;
                map.insert(name, id);
            }
        }
        _ => return Err(anyhow!("JSON phoneme dict must be an array or object")),
    }
    Ok(map)
}
