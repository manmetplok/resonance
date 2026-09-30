//! The provenance sidecar written next to a downloaded model
//! (`<file>.nam.meta.json`): only what the file cannot say about itself.
//! It travels with the file, so a copied `tone3000/` folder keeps its
//! provenance on another machine (nam-model-library.md §4.1).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic_file::{atomic_write, AtomicWriteError};

/// Suffix appended to the model's file name.
pub const SIDECAR_SUFFIX: &str = ".meta.json";

/// `<model>.nam` → `<model>.nam.meta.json`.
pub fn sidecar_path(model: &Path) -> PathBuf {
    let mut name = model.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(SIDECAR_SUFFIX);
    model.with_file_name(name)
}

/// Whether `path` is a sidecar (so a scan never mistakes one for a model).
pub fn is_sidecar(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(SIDECAR_SUFFIX))
}

/// Provenance of one model file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sidecar {
    /// `"tone3000"` for a Tone3000 download.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gear: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    /// Tone3000's size label (`standard`, `feather`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloaded_at: Option<String>,
}

/// The `source` value of a Tone3000 download.
pub const SOURCE_TONE3000: &str = "tone3000";

/// Read the sidecar of `model`, if there is a readable one.
pub fn read_sidecar(model: &Path) -> Option<Sidecar> {
    let bytes = std::fs::read(sidecar_path(model)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Write the sidecar of `model` atomically.
pub fn write_sidecar(model: &Path, sidecar: &Sidecar) -> Result<(), AtomicWriteError> {
    let bytes = serde_json::to_vec_pretty(sidecar).unwrap_or_default();
    atomic_write(&sidecar_path(model), &bytes)
}
