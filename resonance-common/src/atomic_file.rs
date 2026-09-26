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
use std::sync::atomic::{AtomicU64, Ordering};

/// Crash-safe file write: write `bytes` to a sibling `*.tmp` in the
/// same directory, fsync it, atomically rename it over `path`, then
/// fsync the parent directory so the rename itself reaches disk. A
/// crash at any point leaves either the previous file or the new file
/// fully intact — never a truncated target.
///
/// The temp file lives in the same directory as the target so the
/// rename stays within one filesystem (cross-device renames are not
/// atomic). Its name is unique per write — the target's file name plus
/// the process id and a per-process counter, created with `create_new`
/// — so two concurrent writers of the same target (two threads, two app
/// instances) never share, truncate or rename each other's temp file;
/// the last rename wins with one writer's complete content.
///
/// Any failure after the temp file exists (write, fsync or rename — e.g.
/// a full disk) removes it before returning the error. Only a crash can
/// strand one, and a leftover `*.tmp` is inert: it shares no name with
/// any file the loader looks for, so it can never clobber a good target.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| format!("Atomic write target {} has no parent dir", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("Atomic write target {} has no file name", path.display()))?;

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp_path = parent.join(&tmp_name);

    // `create_new` never opens (and truncates) a file that is already
    // there, so even a stale name collision fails loudly instead.
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .map_err(|e| format!("create {}: {e}", tmp_path.display()))?;

    // Write the full contents and fsync before the rename so the new
    // data is durable on disk before it becomes visible at `path`. Then
    // rename — atomic on POSIX: an observer sees either the old or the
    // new file. Any failure removes the temp file so it is not stranded.
    let result = f
        .write_all(bytes)
        .map_err(|e| format!("write {}: {e}", tmp_path.display()))
        .and_then(|()| {
            f.sync_all()
                .map_err(|e| format!("fsync {}: {e}", tmp_path.display()))
        })
        .and_then(|()| {
            drop(f);
            std::fs::rename(&tmp_path, path)
                .map_err(|e| format!("rename {} -> {}: {e}", tmp_path.display(), path.display()))
        });
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

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
