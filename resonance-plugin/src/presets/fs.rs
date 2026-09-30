//! File-system helpers for the preset store: atomic replace, quarantine,
//! file names.
//!
//! The spec routes every write through `resonance_common::atomic_write`.
//! `resonance-plugin` may name only the allow-listed `resonance_common`
//! items (`tools/arch-invariants`, `PLUGIN_COMMON_ITEMS`), and
//! `atomic_file` is not on that list, so this is the same temp + fsync +
//! rename recipe kept local. Swapping it for the shared one is a
//! one-line change once the allow-list grows (plugin-preset-library.md §11).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Write `bytes` to `path` so an observer only ever sees the old file or
/// the whole new one: a uniquely named temp file in the same directory,
/// fsynced, then renamed over the target.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))?;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = parent.join(tmp_name);

    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("write {}: {e}", path.display()));
    }
    if let Ok(dir) = std::fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Move an unparsable preset out of the way as `<name>.corrupt`, so it is
/// neither listed nor silently overwritten.
pub(crate) fn quarantine(path: &Path) -> Option<PathBuf> {
    let mut name = path.file_name()?.to_os_string();
    name.push(".corrupt");
    let to = path.with_file_name(name);
    match std::fs::rename(path, &to) {
        Ok(()) => {
            tracing::warn!("preset {} is unreadable; kept as {}", path.display(), to.display());
            Some(to)
        }
        Err(e) => {
            tracing::error!("quarantine {}: {e}", path.display());
            None
        }
    }
}

/// Keep alphanumerics, `-` and `_`; replace everything else. The display
/// name lives inside the file, so this only has to be a safe file name.
pub(crate) fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `<sanitised name>-<first 8 of id>.json` (§4.2): readable, and unique
/// because the id is. Never recomputed for lookup.
pub(crate) fn preset_file_name(name: &str, id: &str) -> String {
    let stem = sanitize_filename(name);
    let stem = stem.trim_matches('_');
    let short: String = id.chars().filter(|c| *c != '-').take(8).collect();
    format!("{stem}-{short}.json")
}

/// Whether `path` is a candidate preset file (`*.json`, not hidden).
pub(crate) fn is_preset_path(path: &Path) -> bool {
    path.extension().map(|e| e == "json").unwrap_or(false)
        && !path
            .file_name()
            .map(|n| n.to_string_lossy().starts_with('.'))
            .unwrap_or(true)
}
