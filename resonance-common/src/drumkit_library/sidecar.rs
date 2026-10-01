//! The provenance sidecar `kit.meta.json` in a kit's top directory
//! (drums-plugin-rework.md §3.1): only what the kit cannot say about
//! itself. It travels with the directory, so a copied kit keeps its
//! provenance on another machine.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic_file::{atomic_write, AtomicWriteError};

/// The sidecar's file name, in the kit's top directory.
pub const SIDECAR_FILE: &str = "kit.meta.json";

/// `source` of a plok.org download.
pub const SOURCE_PLOK: &str = "plok";
/// `source` of a kit copied in by Import.
pub const SOURCE_IMPORTED: &str = "imported";
/// `source` of anything else (copied in by hand, or migrated without a
/// known origin). A kit with no sidecar is `local` too.
pub const SOURCE_LOCAL: &str = "local";

/// `<kit dir>/kit.meta.json`.
pub fn sidecar_path(kit_dir: &Path) -> PathBuf {
    kit_dir.join(SIDECAR_FILE)
}

/// Provenance of one kit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    /// [`SOURCE_PLOK`], [`SOURCE_IMPORTED`] or [`SOURCE_LOCAL`]. Optional
    /// in the file: a sidecar without it (or with it empty) reads as
    /// `local` and still contributes its other fields.
    #[serde(default)]
    pub source: String,
    /// The kit's name in the plok.org index; the display name when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_name: Option<String>,
    /// The index entry's `file` (the zip's name), the re-download key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_file: Option<String>,
    /// sha256 of the downloaded zip, when the index gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub index_tags: Vec<String>,
    /// RFC 3339: when the kit arrived (downloaded, imported, or the
    /// `installed_at` date of a migrated `installed.json` item).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloaded_at: Option<String>,
    /// Bytes on disk, measured once after extraction or copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

/// Read the sidecar of `kit_dir`, if there is a readable one.
pub fn read_sidecar(kit_dir: &Path) -> Option<Sidecar> {
    let bytes = std::fs::read(sidecar_path(kit_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Write the sidecar of `kit_dir` atomically.
pub fn write_sidecar(kit_dir: &Path, sidecar: &Sidecar) -> Result<(), AtomicWriteError> {
    let bytes = serde_json::to_vec_pretty(sidecar).unwrap_or_default();
    atomic_write(&sidecar_path(kit_dir), &bytes)
}
