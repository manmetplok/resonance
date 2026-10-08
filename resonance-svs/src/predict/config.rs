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

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use crate::config::load_phoneme_dict;
use crate::stages::common::{files_digest, warn_once};

/// Which of the bank's own models a config is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
/// for one it does not (or whose files are missing), and what discovery
/// read of each ([`ModelFacts`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BankModelPaths {
    pub duration: Option<PathBuf>,
    pub pitch: Option<PathBuf>,
    pub variance: Option<PathBuf>,
    /// Per model found: its phonemes, language ids and file digest. A
    /// model without an entry (paths built by hand) is taken to sing every
    /// phoneme and to share the acoustic model's language ids.
    pub facts: BTreeMap<ModelKind, Arc<ModelFacts>>,
}

/// What [`discover`] read of one of the bank's own models.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelFacts {
    /// The symbols its phoneme dictionary has: a unit with any other one
    /// cannot be fed to it.
    pub phonemes: BTreeSet<String>,
    /// The acoustic model's language ids → this model's, matched by the
    /// language names both list; empty when this model lists none. An id
    /// it does not list goes through as it is.
    pub language_map: BTreeMap<i64, i64>,
    /// Its config, phoneme and language files, encoder and model.
    pub files: Vec<PathBuf>,
    /// [`files_digest`] of [`Self::files`]: moves when any of them is
    /// replaced.
    pub files_digest: u64,
}

impl ModelFacts {
    /// This model's language id for the acoustic model's `id`.
    pub fn language(&self, id: i64) -> i64 {
        self.language_map.get(&id).copied().unwrap_or(id)
    }
}

impl BankModelPaths {
    pub fn get(&self, kind: ModelKind) -> Option<&Path> {
        match kind {
            ModelKind::Duration => self.duration.as_deref(),
            ModelKind::Pitch => self.pitch.as_deref(),
            ModelKind::Variance => self.variance.as_deref(),
        }
    }

    fn slot(&mut self, kind: ModelKind) -> &mut Option<PathBuf> {
        match kind {
            ModelKind::Duration => &mut self.duration,
            ModelKind::Pitch => &mut self.pitch,
            ModelKind::Variance => &mut self.variance,
        }
    }

    /// Whether model `kind` is here and knows every symbol of `symbols`.
    pub fn covers<'a>(
        &self,
        kind: ModelKind,
        symbols: impl IntoIterator<Item = &'a str>,
    ) -> bool {
        if self.get(kind).is_none() {
            return false;
        }
        match self.facts.get(&kind) {
            Some(facts) => symbols.into_iter().all(|s| facts.phonemes.contains(s)),
            None => true,
        }
    }

    /// These models less any that does not know every symbol of
    /// `symbols` (the variance model goes with the pitch model it runs
    /// on). What a render unit singing `symbols` can use.
    pub fn covering<'a>(&self, symbols: impl IntoIterator<Item = &'a str> + Clone) -> Self {
        let mut out = self.clone();
        for kind in [ModelKind::Duration, ModelKind::Pitch, ModelKind::Variance] {
            if !self.covers(kind, symbols.clone()) {
                *out.slot(kind) = None;
            }
        }
        if out.pitch.is_none() {
            out.variance = None;
        }
        let kept: Vec<ModelKind> =
            out.facts.keys().copied().filter(|k| out.get(*k).is_some()).collect();
        out.facts.retain(|kind, _| kept.contains(kind));
        out
    }

    /// The facts of model `kind`, if discovery recorded them.
    pub fn facts(&self, kind: ModelKind) -> Option<&ModelFacts> {
        self.facts.get(&kind).map(|f| &**f)
    }
}

/// Find the bank's own models next to `acoustic_config`, whose language
/// names map to ids as `acoustic_languages` says. A model counts when its
/// config parses and names a linguistic encoder and the model, and both
/// files exist. Warnings (a model not used) are logged once per process.
pub fn discover(
    acoustic_config: &Path,
    acoustic_languages: &BTreeMap<String, i64>,
) -> BankModelPaths {
    let Some(dir) = acoustic_config.parent() else {
        return BankModelPaths::default();
    };
    let mut out = BankModelPaths::default();
    for kind in [ModelKind::Duration, ModelKind::Pitch, ModelKind::Variance] {
        let path = dir.join(kind.folder()).join("dsconfig.yaml");
        if !path.is_file() {
            continue;
        }
        let cfg = match ModelConfig::load(&path, kind) {
            Ok(cfg) if cfg.linguistic.is_file() && cfg.model.is_file() => cfg,
            Ok(_) => {
                warn_once(&path, "model files missing; not used".into());
                continue;
            }
            Err(e) => {
                warn_once(&path, format!("{e:#}; not used"));
                continue;
            }
        };
        let files = cfg.files();
        let facts = ModelFacts {
            phonemes: cfg.phonemes.keys().cloned().collect(),
            language_map: language_map(acoustic_languages, &cfg.languages),
            files_digest: files_digest(&files),
            files,
        };
        *out.slot(kind) = Some(path);
        out.facts.insert(kind, Arc::new(facts));
    }
    out
}

/// The acoustic model's language ids → a model's, through the names both
/// list. Empty when the model lists none.
pub fn language_map(
    acoustic: &BTreeMap<String, i64>,
    model: &HashMap<String, i64>,
) -> BTreeMap<i64, i64> {
    acoustic
        .iter()
        .filter_map(|(name, id)| Some((*id, *model.get(name)?)))
        .collect()
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
    pub phonemes_path: PathBuf,
    pub languages: HashMap<String, i64>,
    pub languages_path: Option<PathBuf>,
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
        let languages_path = raw.languages.map(|p| dir.join(p));
        let languages = match &languages_path {
            Some(p) if p.is_file() => {
                let text = std::fs::read_to_string(p)
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
            phonemes_path,
            languages,
            languages_path,
            use_lang_id: raw.use_lang_id.unwrap_or(false),
            speakers: raw.speakers.unwrap_or_default(),
            hop_size: raw.hop_size.unwrap_or(512),
            sample_rate: raw.sample_rate.unwrap_or(44_100),
            hidden_size: raw.hidden_size.unwrap_or(256),
            dir,
        })
    }

    /// The files this model is read from: its config, phoneme and
    /// language files, encoder and model.
    pub fn files(&self) -> Vec<PathBuf> {
        let mut files = vec![self.config_path.clone(), self.linguistic.clone(), self.model.clone()];
        files.push(self.phonemes_path.clone());
        files.extend(self.languages_path.clone());
        files
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
