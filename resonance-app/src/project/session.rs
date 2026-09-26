//! Crash detection for autosave recovery (code review FU-M12a; the ideas
//! of ba todos #466/#467, rebuilt on today's autosave layout).
//!
//! While a session has a project open it keeps a [`SESSION_MARKER`] file
//! in that project's directory — or, for a never-saved project, in its
//! autosave scratch dir. A clean close removes it. A marker that is still
//! there when the project is opened again, left by a process that is no
//! longer running, means that session ended uncleanly; if its
//! [`AUTOSAVE_JSON`] is newer than the canonical [`PROJECT_JSON`], the
//! autosave holds work the last manual save doesn't, and the user is
//! offered it ([`probe`]).
//!
//! Every write here is best effort: a missing marker only costs a
//! recovery prompt, never the session, so I/O errors are logged and
//! swallowed.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::model::{AUTOSAVE_JSON, PROJECT_JSON};

/// File name of the session marker inside a project (or scratch) dir.
pub const SESSION_MARKER: &str = ".session.lock";

/// Contents of a [`SESSION_MARKER`]: which process and session hold the
/// directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMarker {
    pub pid: u32,
    pub session_id: String,
}

/// An autosave that an unclean exit left behind, newer than the last
/// manual save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryOffer {
    /// The directory holding the marker and the autosave: the project dir,
    /// or an untitled session's scratch dir.
    pub dir: PathBuf,
    /// When the autosave was written.
    pub autosave_at: SystemTime,
    /// When `project.json` was last written; `None` for a project that was
    /// never saved (the autosave is the only copy).
    pub saved_at: Option<SystemTime>,
}

impl RecoveryOffer {
    /// The snapshot to recover: `{dir}/project.autosave.json`.
    pub fn autosave_json(&self) -> PathBuf {
        self.dir.join(AUTOSAVE_JSON)
    }
}

/// Write `dir`'s marker, claiming it for this process and `session_id`.
pub fn write_marker(dir: &Path, session_id: &str) {
    let marker = SessionMarker {
        pid: std::process::id(),
        session_id: session_id.to_owned(),
    };
    let result = serde_json::to_vec(&marker)
        .map_err(|e| e.to_string())
        .and_then(|bytes| {
            super::io::atomic_write(&dir.join(SESSION_MARKER), &bytes).map_err(|e| e.to_string())
        });
    if let Err(e) = result {
        tracing::warn!("session marker write in {} failed: {e}", dir.display());
    }
}

/// Remove `dir`'s marker. A missing marker is not an error.
pub fn remove_marker(dir: &Path) {
    match std::fs::remove_file(dir.join(SESSION_MARKER)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("session marker remove in {} failed: {e}", dir.display()),
    }
}

/// Read `dir`'s marker. `None` when there is none; a marker that exists
/// but can't be parsed (a crash mid-write) reads as held by no live
/// process.
pub fn read_marker(dir: &Path) -> Option<SessionMarker> {
    let bytes = std::fs::read(dir.join(SESSION_MARKER)).ok()?;
    Some(serde_json::from_slice(&bytes).unwrap_or(SessionMarker {
        pid: 0,
        session_id: String::new(),
    }))
}

/// Whether `marker` belongs to a session that is still running — this one
/// (`own_session_id`) or another live process. Only Linux can tell for
/// another process (`/proc/<pid>`); elsewhere a foreign marker counts as
/// dead, which at worst offers a recovery the user can decline.
pub fn marker_is_live(marker: &SessionMarker, own_session_id: &str) -> bool {
    if marker.session_id == own_session_id {
        return true;
    }
    // Our own pid under another session id: the previous owner died and
    // the pid was reused by us.
    if marker.pid == 0 || marker.pid == std::process::id() {
        return false;
    }
    cfg!(target_os = "linux") && Path::new(&format!("/proc/{}", marker.pid)).exists()
}

/// The pure recovery rule: an unclean exit (a marker no live session
/// holds) with an autosave strictly newer than the last manual save. No
/// `project.json` at all means the autosave is the only copy.
pub fn recoverable_from_state(
    unclean_exit: bool,
    autosave: Option<SystemTime>,
    saved: Option<SystemTime>,
) -> bool {
    match (unclean_exit, autosave, saved) {
        (true, Some(auto), Some(saved)) => auto > saved,
        (true, Some(_), None) => true,
        _ => false,
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Whether `dir` holds a recoverable autosave (see
/// [`recoverable_from_state`]). `own_session_id` is the asking session:
/// its own marker is never a crash.
pub fn probe(dir: &Path, own_session_id: &str) -> Option<RecoveryOffer> {
    let marker = read_marker(dir)?;
    let unclean = !marker_is_live(&marker, own_session_id);
    let autosave_at = mtime(&dir.join(AUTOSAVE_JSON));
    let saved_at = mtime(&dir.join(PROJECT_JSON));
    recoverable_from_state(unclean, autosave_at, saved_at).then(|| RecoveryOffer {
        dir: dir.to_path_buf(),
        autosave_at: autosave_at.unwrap_or(SystemTime::UNIX_EPOCH),
        saved_at,
    })
}

/// The newest recoverable untitled session among the scratch dirs under
/// `root` (one per session), or `None`.
pub fn scan_scratch_root(root: &Path, own_session_id: &str) -> Option<RecoveryOffer> {
    std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|dir| probe(&dir, own_session_id))
        .max_by_key(|offer| offer.autosave_at)
}
