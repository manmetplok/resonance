//! Crash-safe file writes, shared by every user-state store that
//! persists a single JSON (or other) file outside the project format
//! (the installed-content registry, controller-map presets, app
//! settings, the recent-projects list, track presets).
//!
//! Mirrors `resonance-app::project::io::atomic_write` (doc #171,
//! todo #461), which pioneered this write-temp-then-rename pattern for
//! project saves; this is the same primitive lifted somewhere both
//! `resonance-common` and `resonance-app` can reach it, so user-state
//! writes get the same crash safety a project save already has.

use std::io::Write;
use std::path::Path;

/// Crash-safe file write: write `bytes` to a sibling `*.tmp` in the
/// same directory, fsync it, atomically rename it over `path`, then
/// fsync the parent directory so the rename itself reaches disk. A
/// crash at any point leaves either the previous file or the new file
/// fully intact — never a truncated target.
///
/// The temp file lives in the same directory as the target so the
/// rename stays within one filesystem (cross-device renames are not
/// atomic). Its name embeds the target's file name to avoid colliding
/// with the temp files of sibling writes in the same directory.
///
/// A leftover `*.tmp` from an interrupted write is inert: it shares no
/// name with any file the loader looks for, so it can never clobber a
/// good target file.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| format!("Atomic write target {} has no parent dir", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("Atomic write target {} has no file name", path.display()))?;

    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = parent.join(&tmp_name);

    // Write the full contents and fsync before the rename so the new
    // data is durable on disk before it becomes visible at `path`.
    {
        let mut f = std::fs::File::create(&tmp_path)
            .map_err(|e| format!("create {}: {e}", tmp_path.display()))?;
        f.write_all(bytes)
            .map_err(|e| format!("write {}: {e}", tmp_path.display()))?;
        f.sync_all()
            .map_err(|e| format!("fsync {}: {e}", tmp_path.display()))?;
    }

    // Atomic on POSIX: an observer sees either the old or the new file.
    std::fs::rename(&tmp_path, path).map_err(|e| {
        // Best-effort cleanup so a failed rename doesn't strand the tmp.
        let _ = std::fs::remove_file(&tmp_path);
        format!("rename {} -> {}: {e}", tmp_path.display(), path.display())
    })?;

    // fsync the directory so the rename entry itself survives a crash.
    // Directory fsync is unsupported on some platforms/filesystems, so
    // failures here are tolerated — the file data is already durable.
    if let Ok(dir) = std::fs::File::open(parent) {
        let _ = dir.sync_all();
    }

    Ok(())
}

/// Preserve a corrupt user-state file as `<name>.corrupt` (best effort)
/// so a parse failure never silently destroys the user's data: the next
/// save would otherwise overwrite it with a fresh default. Called by a
/// loader right before it falls back to a default value.
///
/// Renames rather than copies, so a failure here (e.g. no write
/// permission) is the only way to lose the original — the common case
/// just relocates the file. Overwrites any previous `.corrupt` from an
/// earlier failed load, since that older one already had its chance to
/// be noticed.
pub fn quarantine_corrupt(path: &Path) {
    let mut corrupt_name = match path.file_name() {
        Some(n) => n.to_os_string(),
        None => return,
    };
    corrupt_name.push(".corrupt");
    let corrupt_path = path.with_file_name(corrupt_name);
    if let Err(e) = std::fs::rename(path, &corrupt_path) {
        eprintln!(
            "quarantine corrupt file {} -> {}: {e}",
            path.display(),
            corrupt_path.display()
        );
    }
}
