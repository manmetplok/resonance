//! User voice presets on disk: one JSON file per preset under
//! `$XDG_DATA_HOME/resonance/voice-presets/` (`$RESONANCE_VOICE_PRESET_DIR`
//! overrides it; a hermetic test app gets a private folder of its own).
//! File names use the track presets' injective encoding, so two names
//! never share a file.

use std::path::{Path, PathBuf};

use resonance_common::{atomic_write, quarantine_corrupt};
use serde::{Deserialize, Serialize};

use super::{user_id, VoicePreset, VoiceSettings};

/// Environment override for the user voice preset folder.
pub const VOICE_PRESET_DIR_ENV: &str = "RESONANCE_VOICE_PRESET_DIR";

/// The folder a machine (not a test) keeps user voice presets in:
/// `override_dir` (the value of [`VOICE_PRESET_DIR_ENV`]) when set, else
/// `<data dir>/resonance/voice-presets`.
pub fn machine_dir(override_dir: Option<PathBuf>, data_dir: Option<PathBuf>) -> Option<PathBuf> {
    override_dir.or_else(|| data_dir.map(|d| d.join("resonance/voice-presets")))
}

/// What a user preset file holds. The id is derived from the name and
/// the score is never stored (only the built-ins carry one).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PresetFile {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    settings: VoiceSettings,
}

/// The user voice presets in one folder.
#[derive(Debug, Clone, Default)]
pub struct VoicePresetStore {
    dir: Option<PathBuf>,
}

impl VoicePresetStore {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    fn file(&self, name: &str) -> Option<PathBuf> {
        let dir = self.dir.as_ref()?;
        Some(dir.join(format!("{}.json", crate::presets::encode_filename(name))))
    }

    /// Every readable preset, by name. A file that does not parse is
    /// quarantined as `.corrupt` (never overwritten by a later save).
    pub fn load(&self) -> Vec<VoicePreset> {
        let Some(dir) = &self.dir else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut presets = Vec::new();
        for path in entries.flatten().map(|e| e.path()) {
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            match read(&path) {
                Ok(preset) => presets.push(preset),
                Err(e) => {
                    tracing::warn!(
                        "voice preset {} is corrupt ({e}); quarantining it",
                        path.display()
                    );
                    quarantine_corrupt(&path);
                }
            }
        }
        presets.sort_by_key(|p| p.name.to_lowercase());
        presets
    }

    /// Whether a user preset by this name exists.
    pub fn exists(&self, name: &str) -> bool {
        self.file(name)
            .is_some_and(|path| read(&path).is_ok_and(|p| p.name == name))
    }

    /// Write `preset` (its name, description, tags and settings), replacing
    /// a preset of the same name. Refuses an empty name and a file that
    /// holds a different preset (a case-insensitive filesystem folds two
    /// names onto one file). Callers confirm an overwrite first.
    pub fn save(&self, preset: &VoicePreset) -> Result<PathBuf, String> {
        let name = preset.name.trim();
        if name.is_empty() {
            return Err("name a voice preset before saving it".to_owned());
        }
        let path = self
            .file(name)
            .ok_or_else(|| "could not determine the voice preset folder".to_owned())?;
        if path.exists() {
            match read(&path) {
                Ok(existing) if existing.name == name => {}
                Ok(existing) => {
                    return Err(format!(
                        "{} already holds the voice preset {:?}; save under another name",
                        path.display(),
                        existing.name
                    ))
                }
                Err(e) => {
                    return Err(format!(
                        "{} exists and is not a readable voice preset ({e}); not replacing it",
                        path.display()
                    ))
                }
            }
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        let file = PresetFile {
            name: name.to_owned(),
            description: preset.description.clone(),
            tags: preset.tags.clone(),
            settings: preset.settings.clone(),
        };
        let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        atomic_write(&path, json.as_bytes()).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// Remove the preset named `name`. Not undoable.
    pub fn delete(&self, name: &str) -> Result<(), String> {
        let Some(path) = self.file(name) else {
            return Err("could not determine the voice preset folder".to_owned());
        };
        match read(&path) {
            Ok(p) if p.name == name => {
                std::fs::remove_file(&path).map_err(|e| format!("delete voice preset: {e}"))
            }
            _ => Err(format!("no voice preset named {name:?}")),
        }
    }

    /// Rename `from` to `to`; refuses a `to` that another preset holds.
    pub fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        let to = to.trim();
        if to.is_empty() {
            return Err("a voice preset needs a name".to_owned());
        }
        if to == from {
            return Ok(());
        }
        let source = self
            .file(from)
            .ok_or_else(|| "could not determine the voice preset folder".to_owned())?;
        let mut preset = match read(&source) {
            Ok(p) if p.name == from => p,
            _ => return Err(format!("no voice preset named {from:?}")),
        };
        if self.exists(to) {
            return Err(format!("a voice preset named {to:?} already exists"));
        }
        preset.name = to.to_owned();
        let target = self.save(&preset)?;
        if target != source {
            std::fs::remove_file(&source).map_err(|e| format!("rename voice preset: {e}"))?;
        }
        Ok(())
    }
}

fn read(path: &Path) -> Result<VoicePreset, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let file: PresetFile = serde_json::from_str(&json).map_err(|e| format!("parse: {e}"))?;
    file.settings.validate()?;
    Ok(VoicePreset {
        id: user_id(&file.name),
        name: file.name,
        description: file.description,
        tags: file.tags,
        score: None,
        builtin: false,
        settings: file.settings,
    })
}
