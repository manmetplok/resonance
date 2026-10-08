//! Where a bank keeps its own duration, pitch and variance models, and
//! what each model's `dsconfig.yaml` says about them.
//!
//! OpenUtau's layout: next to the bank's acoustic `dsconfig.yaml` sit
//! `dsdur/`, `dspitch/` and `dsvariance/`, each with a `dsconfig.yaml`
//! naming a `linguistic` encoder and the `dur` / `pitch` / `variance`
//! model, its own phoneme dictionary (same symbols as the acoustic model,
//! different ids), its `languages`, and its own `speakers` (embeddings
//! relative to that folder; the duration model of a multi-speaker bank
//! may have only one).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use crate::config::load_phoneme_dict;

/// Which of the bank's own models a config is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelKind {
    Duration,
    Pitch,
    Variance,
}

impl ModelKind {
    /// The folder next to the acoustic config (`dsdur`, `dspitch`,
    /// `dsvariance`).
    pub fn folder(self) -> &'static str {
        match self {
            ModelKind::Duration => "dsdur",
            ModelKind::Pitch => "dspitch",
            ModelKind::Variance => "dsvariance",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ModelKind::Duration => "duration",
            ModelKind::Pitch => "pitch",
            ModelKind::Variance => "variance",
        }
    }
}

/// The `dsconfig.yaml` of each of the bank's own models it ships, `None`
/// for one it does not (or whose files are missing).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BankModelPaths {
    pub duration: Option<PathBuf>,
    pub pitch: Option<PathBuf>,
    pub variance: Option<PathBuf>,
}

impl BankModelPaths {
    pub fn get(&self, kind: ModelKind) -> Option<&Path> {
        match kind {
            ModelKind::Duration => self.duration.as_deref(),
            ModelKind::Pitch => self.pitch.as_deref(),
            ModelKind::Variance => self.variance.as_deref(),
        }
    }
}

/// Find the bank's own models next to `acoustic_config`. A model counts
/// when its config parses and names a linguistic encoder and the model,
/// and both files exist.
pub fn discover(acoustic_config: &Path) -> BankModelPaths {
    let Some(dir) = acoustic_config.parent() else {
        return BankModelPaths::default();
    };
    let find = |kind: ModelKind| {
        let path = dir.join(kind.folder()).join("dsconfig.yaml");
        if !path.is_file() {
            return None;
        }
        match ModelConfig::load(&path, kind) {
            Ok(cfg) if cfg.linguistic.is_file() && cfg.model.is_file() => Some(path),
            Ok(_) => {
                tracing::warn!("{}: model files missing; not used", path.display());
                None
            }
            Err(e) => {
                tracing::warn!("{}: {e:#}; not used", path.display());
                None
            }
        }
    };
    BankModelPaths {
        duration: find(ModelKind::Duration),
        pitch: find(ModelKind::Pitch),
        variance: find(ModelKind::Variance),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawModelConfig {
    phonemes: Option<String>,
    languages: Option<String>,
    use_lang_id: Option<bool>,
    linguistic: Option<String>,
    dur: Option<String>,
    pitch: Option<String>,
    variance: Option<String>,
    speakers: Option<Vec<String>>,
    hop_size: Option<i32>,
    sample_rate: Option<i32>,
    hidden_size: Option<usize>,
}

/// One of the bank's own models, as its `dsconfig.yaml` describes it.
#[derive(Debug, Clone)]
pub struct ModelConfig {
    pub kind: ModelKind,
    /// The `dsconfig.yaml` it was read from.
    pub config_path: PathBuf,
    /// Its folder: speaker embeddings resolve against it.
    pub dir: PathBuf,
    pub linguistic: PathBuf,
    /// The `dur` / `pitch` / `variance` model.
    pub model: PathBuf,
    pub phonemes: HashMap<String, i64>,
    pub languages: HashMap<String, i64>,
    pub use_lang_id: bool,
    /// Speaker keys (`files/tiger_disco`, `embeds/standard`), relative to
    /// [`Self::dir`], without `.emb`.
    pub speakers: Vec<String>,
    pub hop_size: i32,
    pub sample_rate: i32,
    pub hidden_size: usize,
}

impl ModelConfig {
    pub fn load(path: &Path, kind: ModelKind) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let raw: RawModelConfig = serde_yaml_ng::from_str(&text)
            .with_context(|| format!("parsing {}", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let rel = |key: &str, v: Option<String>| {
            v.map(|p| dir.join(p))
                .ok_or_else(|| anyhow!("{} names no `{key}`", path.display()))
        };
        let model = match kind {
            ModelKind::Duration => rel("dur", raw.dur)?,
            ModelKind::Pitch => rel("pitch", raw.pitch)?,
            ModelKind::Variance => rel("variance", raw.variance)?,
        };
        let phonemes_path = rel("phonemes", raw.phonemes)?;
        let phonemes = load_phoneme_dict(&phonemes_path)?;
        let languages = match raw.languages.map(|p| dir.join(p)) {
            Some(p) if p.is_file() => {
                let text = std::fs::read_to_string(&p)
                    .with_context(|| format!("reading {}", p.display()))?;
                serde_json::from_str(&text).with_context(|| format!("parsing {}", p.display()))?
            }
            _ => HashMap::new(),
        };
        Ok(Self {
            kind,
            config_path: path.to_path_buf(),
            linguistic: rel("linguistic", raw.linguistic)?,
            model,
            phonemes,
            languages,
            use_lang_id: raw.use_lang_id.unwrap_or(false),
            speakers: raw.speakers.unwrap_or_default(),
            hop_size: raw.hop_size.unwrap_or(512),
            sample_rate: raw.sample_rate.unwrap_or(44_100),
            hidden_size: raw.hidden_size.unwrap_or(256),
            dir,
        })
    }

    /// Seconds per frame.
    pub fn frame_sec(&self) -> f64 {
        self.hop_size as f64 / self.sample_rate as f64
    }

    /// The model's own speaker for the acoustic speaker key `acoustic`
    /// (`dsacoustic/tiger_disco`, `embeds/phantom`): the one whose last
    /// path segment matches, else its first (a duration model trained on
    /// one voice serves every mode). `None` when it has no speakers.
    pub fn speaker_for(&self, acoustic: Option<&str>) -> Option<&str> {
        let last = |s: &str| s.rsplit(['/', '\\']).next().unwrap_or(s).to_owned();
        let wanted = acoustic.map(last);
        self.speakers
            .iter()
            .find(|s| wanted.as_deref() == Some(last(s).as_str()))
            .or_else(|| self.speakers.first())
            .map(String::as_str)
    }
}
