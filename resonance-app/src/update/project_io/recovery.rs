//! Crash detection + autosave recovery (code review FU-M12a; ba todos
//! #466/#467 on `ba/epic-32`, rebuilt on today's autosave layout).
//!
//! The on-disk rule lives in [`crate::project::session`]. This module
//! keeps the session marker in step with the open project: it follows the
//! project through open / new / Save As, sits in the untitled scratch dir
//! once an untitled project has an autosave, and is removed on a clean
//! quit.

use crate::project::session;
use crate::Resonance;

/// Where this session's marker belongs right now: the project dir, or —
/// for an untitled project that has autosaved — its scratch dir.
fn desired_marker_dir(r: &Resonance) -> Option<std::path::PathBuf> {
    match &r.io.project_path {
        Some(path) => Some(path.clone()),
        None => super::autosave_scratch_dir(r)
            .filter(|dir| dir.join(crate::project::AUTOSAVE_JSON).exists()),
    }
}

/// Move the marker to where it belongs (see [`desired_marker_dir`]). Called
/// whenever the project's location may have changed: a load, a new
/// project, a completed save or autosave.
pub(crate) fn sync_session_marker(r: &mut Resonance) {
    let desired = desired_marker_dir(r);
    if desired == r.io.session_marker_dir {
        return;
    }
    if let Some(old) = r.io.session_marker_dir.take() {
        session::remove_marker(&old);
    }
    if let Some(dir) = &desired {
        if dir.is_dir() {
            session::write_marker(dir, r.session_id());
            r.io.session_marker_dir = desired;
        }
    }
}

/// A clean end of the session — the window closes with nothing unsaved,
/// or the user chose Save & Quit / Discard & Quit. Drop the marker so the
/// next open offers no recovery, and delete this session's untitled
/// scratch dir: its work was either saved elsewhere or discarded.
pub(crate) fn close_session(r: &mut Resonance) {
    if let Some(dir) = r.io.session_marker_dir.take() {
        session::remove_marker(&dir);
    }
    if let Some(scratch) = super::autosave_scratch_dir(r) {
        remove_scratch_dir(&scratch);
    }
}

/// A completed save (`autosave` false) or autosave. A manual save that
/// gave an untitled project its path leaves the scratch dir's autosave
/// superseded, so it is deleted.
pub(crate) fn after_save(r: &mut Resonance, autosave: bool) {
    sync_session_marker(r);
    if !autosave && r.io.project_path.is_some() {
        if let Some(scratch) = super::autosave_scratch_dir(r) {
            remove_scratch_dir(&scratch);
        }
    }
}

fn remove_scratch_dir(dir: &std::path::Path) {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("remove autosave scratch dir {}: {e}", dir.display()),
    }
}
