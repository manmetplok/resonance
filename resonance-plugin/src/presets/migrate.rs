//! The one-shot converter from pre-format-1 user presets
//! (plugin-preset-library.md §13).
//!
//! There are no real users yet, so this is a converter, not a
//! compatibility layer. A legacy file is a bare state document —
//! `{version, params, …extra}` as the editor wrote it, or the full state
//! blob the host wrote, `"preset"` session key and all — with the display
//! name stored under a top-level `"name"` (or nothing, for a file dropped
//! in by hand).
//!
//! Each one becomes a format-1 file: the document goes into `state.doc`
//! with `"name"` and `"preset"` stripped, a UUID is minted,
//! `meta.name` is the stored name (or the file stem), `meta.category` is
//! parsed from a `"<X> — <Y>"` / `"<X> - <Y>"` / `"<X>___<Y>"` prefix
//! when `X` is a seeded category, and `created`/`modified` are the old
//! file's mtime. The id is derived from the plugin id, the file name and
//! the file's bytes ([`format::derived_uuid`]), so two processes racing to
//! convert one file write the same id to the same `<stem>-<id8>.json`
//! rather than two copies. The original is kept beside it as
//! `<file>.legacy`, stamped with the conversion time, and removed by a
//! later run once it is older than [`LEGACY_RETENTION`]. Files that
//! already carry `"format"` are skipped (a missing id is filled in by the
//! index scan), so running this twice is a no-op.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::format::{self, PresetFile, PresetMeta, PresetPluginInfo};
use super::{fs, vocab, PRESET_STATE_KEY};

/// What [`convert_legacy_dir`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConvertReport {
    /// `(legacy file, new file)`.
    pub converted: Vec<(PathBuf, PathBuf)>,
    /// Unparsable files moved aside as `*.corrupt`.
    pub quarantined: Vec<PathBuf>,
    /// `.legacy` backups from an earlier run that were removed.
    pub backups_removed: usize,
}

impl ConvertReport {
    pub fn is_empty(&self) -> bool {
        self.converted.is_empty() && self.quarantined.is_empty() && self.backups_removed == 0
    }
}

/// Suffix the converted original is kept under.
pub const LEGACY_SUFFIX: &str = ".legacy";

/// How long a `.legacy` backup is kept after its conversion.
pub const LEGACY_RETENTION: std::time::Duration =
    std::time::Duration::from_secs(30 * 24 * 60 * 60);

/// Convert every legacy preset in `dir` (one plugin's directory). The
/// library runs this once per directory per process, before its first
/// scan.
pub fn convert_legacy_dir(dir: &Path, plugin_id: &str, now: SystemTime) -> ConvertReport {
    let mut report = ConvertReport::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return report;
    };
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(LEGACY_SUFFIX) {
            let age = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| now.duration_since(m).ok());
            let expired = age.is_some_and(|a| a > LEGACY_RETENTION);
            if expired && std::fs::remove_file(&path).is_ok() {
                report.backups_removed += 1;
            }
        } else if fs::is_preset_path(&path) {
            candidates.push(path);
        }
    }
    candidates.sort();
    for path in candidates {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        match serde_json::from_str::<serde_json::Value>(&text) {
            Err(_) => {
                if let Some(q) = fs::quarantine(&path) {
                    report.quarantined.push(q);
                }
            }
            Ok(v) if format::is_envelope(&v) => {}
            Ok(_) => match convert_legacy_file(&path, plugin_id, now) {
                Ok((to, _)) => report.converted.push((path, to)),
                Err(e) => tracing::warn!("convert {}: {e}", path.display()),
            },
        }
    }
    report
}

/// Convert one legacy file in place (see the module docs). Returns the
/// new path and the file written.
pub fn convert_legacy_file(
    path: &Path,
    plugin_id: &str,
    now: SystemTime,
) -> Result<(PathBuf, PresetFile), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let doc: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("not JSON: {e}"))?;
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(now);
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let id = format::derived_uuid(&[
        plugin_id.as_bytes(),
        file_name.as_bytes(),
        text.as_bytes(),
    ]);
    let file =
        convert_legacy_document(doc, &super::library::stem_of(path), plugin_id, mtime, id)?;
    let dir = path
        .parent()
        .ok_or_else(|| "preset has no directory".to_string())?;
    let to = dir.join(fs::preset_file_name(&file.meta.name, &file.id));
    fs::atomic_write(&to, file.to_text()?.as_bytes())?;
    let mut backup = path.as_os_str().to_os_string();
    backup.push(LEGACY_SUFFIX);
    match std::fs::rename(path, &backup) {
        // Stamp the backup with the conversion time: its retention runs
        // from now, not from when the original was last edited.
        Ok(()) => {
            if let Ok(f) = std::fs::File::options().write(true).open(&backup) {
                let _ = f.set_modified(now);
            }
        }
        // The converted file exists; a leftover original would be
        // converted again next start, so remove it rather than duplicate.
        // (A racing converter may already have moved it: also fine.)
        Err(e) => {
            if path.exists() {
                tracing::warn!("keep {} as .legacy: {e}", path.display());
                let _ = std::fs::remove_file(path);
            }
        }
    }
    Ok((to, file))
}

/// The format-1 file a legacy state document becomes, with id `id`.
/// `fallback_name` (the file stem) names a document that stored none.
pub fn convert_legacy_document(
    mut doc: serde_json::Value,
    fallback_name: &str,
    plugin_id: &str,
    stamped: SystemTime,
    id: String,
) -> Result<PresetFile, String> {
    let obj = doc
        .as_object_mut()
        .ok_or_else(|| "a preset must be a JSON object".to_string())?;
    let stored = obj
        .remove("name")
        .and_then(|n| n.as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty());
    obj.remove(PRESET_STATE_KEY);
    if !obj.contains_key("params") {
        return Err("no params object".to_string());
    }
    let name = stored.unwrap_or_else(|| fallback_name.to_string());
    if plugin_id == "com.resonance.amp"
        && obj.get("params").and_then(|p| p.get("file_select")).is_some()
        && !obj.contains_key("model_path")
    {
        tracing::info!(
            "amp preset '{name}' carries only a file_select index and no model reference"
        );
    }
    let when = format::rfc3339(stamped);
    let meta = PresetMeta {
        category: vocab::category_from_name(&name).map(str::to_string),
        created: Some(when.clone()),
        modified: Some(when),
        ..PresetMeta::named(name)
    };
    Ok(PresetFile::new(
        id,
        PresetPluginInfo {
            id: plugin_id.to_string(),
            ..PresetPluginInfo::default()
        },
        meta.normalized(),
        doc,
    ))
}
