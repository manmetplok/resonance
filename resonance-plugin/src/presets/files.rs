//! File names and writes for the preset store. The atomic replace and the
//! quarantine are the shared `resonance_common::atomic_file` ones (the
//! same recipe the marks store and track presets use); what is here is
//! only the preset library's own naming.

use std::path::{Path, PathBuf};

use resonance_common::atomic_file;

/// Write `bytes` to `path` so an observer only ever sees the old file or
/// the whole new one ([`atomic_file::atomic_write`]).
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    atomic_file::atomic_write(path, bytes).map_err(|e| e.to_string())
}

/// Move an unparsable preset out of the way as `<name>.corrupt`
/// ([`atomic_file::quarantine_corrupt`]), so it is neither listed nor
/// silently overwritten. Returns where it went, if the move happened.
pub(crate) fn quarantine(path: &Path) -> Option<PathBuf> {
    let mut name = path.file_name()?.to_os_string();
    name.push(".corrupt");
    let to = path.with_file_name(name);
    atomic_file::quarantine_corrupt(path);
    to.exists().then_some(to)
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

/// Move the file at `from` to `to` before `to` is rewritten, so a preset
/// whose file name changes (rename, a re-save under a new spelling) is
/// never briefly two files, and a *case-only* change is safe on a
/// case-insensitive filesystem (APFS, exFAT, casefold ext4). There
/// `from` and `to` are one file, and "write `to`, then delete `from`"
/// would delete the preset just written. `rename` handles both: it is a
/// no-op or an in-place re-case for the same file, and a move otherwise.
pub(crate) fn move_before_rewrite(from: &Path, to: &Path) -> Result<(), String> {
    if from == to || !from.exists() {
        return Ok(());
    }
    std::fs::rename(from, to)
        .map_err(|e| format!("move {} to {}: {e}", from.display(), to.display()))
}
