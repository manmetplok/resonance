//! Crash detection + autosave recovery (code review FU-M12a; ba todos
//! #466/#467 on `ba/epic-32`, rebuilt on today's autosave layout).
//!
//! The on-disk rule lives in [`crate::project::session`]. This module
//! keeps the session marker in step with the open project: it follows the
//! project through open / new / Save As, sits in the untitled scratch dir
//! once an untitled project has an autosave, and is removed on a clean
//! quit.

use std::path::{Path, PathBuf};

use iced::Task;

use crate::message::{Message, RecoveryChoice};
use crate::project::session;
use crate::state::{LoadRecovery, RecoveryPrompt};
use crate::Resonance;

/// The directory an open targets: a project dir, or the dir of a JSON
/// file inside it.
fn project_dir_of(path: &Path) -> PathBuf {
    if path.is_file() {
        path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

/// A GUI open of `path`: when the project holds an autosave an unclean
/// exit left behind, open the recovery prompt instead of loading, and
/// return `true`. The prompt's answer arrives as
/// [`crate::message::ProjectIoMessage::RecoveryChoice`].
pub(crate) fn prompt_before_open(r: &mut Resonance, path: &Path) -> bool {
    match session::probe(&project_dir_of(path), r.session_id()) {
        Some(offer) => {
            r.io.recovery_prompt = Some(RecoveryPrompt {
                offer,
                untitled: false,
            });
            true
        }
        None => false,
    }
}

/// At startup: offer the newest crashed untitled session found in the
/// autosave scratch root, if any. Nothing else is open yet, so the prompt
/// sits over the startup screen.
pub(crate) fn offer_orphaned_session(r: &mut Resonance) {
    let Some(root) = super::autosave_scratch_root() else {
        return;
    };
    // First drop marker-less scratch dirs nobody touched for a week: no
    // prompt would ever offer them (FU-R1a).
    session::gc_scratch_root(
        &root,
        r.session_id(),
        session::SCRATCH_GC_AGE,
        std::time::SystemTime::now(),
    );
    if let Some(offer) = session::scan_scratch_root(&root, r.session_id()) {
        r.io.recovery_prompt = Some(RecoveryPrompt {
            offer,
            untitled: true,
        });
    }
}

/// Start the async load of `target` as a token-tagged open (FU-A1a: a
/// later open makes this one's result stale), recording what it means for
/// recovery. `open_dir` fills the pending-open slot and becomes the
/// project path once the load succeeds — except for an untitled recovery,
/// whose slot holds its scratch dir and is never adopted
/// ([`loads_untitled`]).
fn start_load(
    r: &mut Resonance,
    target: PathBuf,
    open_dir: PathBuf,
    recovery: LoadRecovery,
) -> Task<Message> {
    r.io.open_token = r.io.open_token.wrapping_add(1);
    r.io.pending_open_path = Some(open_dir);
    r.io.load_recovery = Some(recovery);
    super::dialogs::load_project_task(target, r.io.open_token)
}

/// The open in flight recovers a crashed untitled session: it lands
/// untitled, so its pending slot (the scratch dir) is not adopted.
pub(crate) fn loads_untitled(r: &Resonance) -> bool {
    r.io
        .load_recovery
        .as_ref()
        .is_some_and(|l| l.scratch_dir.is_some())
}

/// The user's answer to the recovery prompt.
pub(crate) fn handle_choice(r: &mut Resonance, choice: RecoveryChoice) -> Task<Message> {
    let Some(prompt) = r.io.recovery_prompt.take() else {
        return Task::none();
    };
    let offer = prompt.offer;
    match (choice, prompt.untitled) {
        (RecoveryChoice::Cancel, _) => Task::none(),
        (_, _) if r.refuse_project_switch_during_render() => Task::none(),
        (RecoveryChoice::RecoverAutosave, false) => {
            let recovery = LoadRecovery {
                recovered: true,
                autosave_available: true,
                scratch_dir: None,
            };
            start_load(r, offer.autosave_json(), offer.dir.clone(), recovery)
        }
        (RecoveryChoice::RecoverAutosave, true) => {
            // Offered only at startup, before anything is open: the
            // recovered session lands untitled.
            r.io.project_path = None;
            let recovery = LoadRecovery {
                recovered: true,
                autosave_available: true,
                scratch_dir: Some(offer.dir.clone()),
            };
            start_load(r, offer.autosave_json(), offer.dir.clone(), recovery)
        }
        (RecoveryChoice::OpenLastSaved, false) => {
            let recovery = LoadRecovery {
                recovered: false,
                autosave_available: true,
                scratch_dir: None,
            };
            start_load(r, offer.dir.clone(), offer.dir.clone(), recovery)
        }
        (RecoveryChoice::Discard, true) => {
            remove_scratch_dir(&offer.dir);
            Task::none()
        }
        // A button the prompt doesn't show for this subject.
        (RecoveryChoice::OpenLastSaved, true) | (RecoveryChoice::Discard, false) => Task::none(),
    }
}

/// Open `path` with the recovery decision already made (control
/// `project.open`): `recover` loads a recoverable autosave; otherwise, or
/// when there is none, the last saved version loads. The job result
/// reports both facts (see [`finish_load`]).
pub(crate) fn open_resolved(r: &mut Resonance, path: PathBuf, recover: bool) -> Task<Message> {
    if r.refuse_project_switch_during_render() {
        return Task::none();
    }
    match session::probe(&project_dir_of(&path), r.session_id()) {
        Some(offer) if recover => {
            let recovery = LoadRecovery {
                recovered: true,
                autosave_available: true,
                scratch_dir: None,
            };
            start_load(r, offer.autosave_json(), offer.dir.clone(), recovery)
        }
        offer => {
            let recovery = LoadRecovery {
                recovered: false,
                autosave_available: offer.is_some(),
                scratch_dir: None,
            };
            start_load(r, path.clone(), path, recovery)
        }
    }
}

/// A disk load has replayed: apply what its [`LoadRecovery`] says and
/// return it for the control job's result. A recovered autosave lands
/// dirty, so a normal save writes it to the canonical files; the autosave
/// itself is left alone.
pub(crate) fn finish_load(r: &mut Resonance) -> LoadRecovery {
    let recovery = r.io.load_recovery.take().unwrap_or_default();
    if recovery.recovered {
        r.session.dirty = true;
    }
    if let Some(scratch) = &recovery.scratch_dir {
        // Claim the crashed session's marker: a second crash before the
        // next save offers it again.
        session::write_marker(scratch, r.session_id());
        r.io.recovered_scratch_dir = Some(scratch.clone());
    }
    recovery
}

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
    if let Some(recovered) = r.io.recovered_scratch_dir.take() {
        remove_scratch_dir(&recovered);
    }
}

/// A completed save (`autosave` false) or autosave. A manual save that
/// gave an untitled project its path leaves the scratch dir's autosave
/// superseded, so it is deleted — as is a recovered crashed session's,
/// whose work is now saved — and so is the project dir's own autosave
/// when it is older than the save (FU-R1a).
pub(crate) fn after_save(r: &mut Resonance, autosave: bool) {
    sync_session_marker(r);
    if !autosave {
        if let Some(dir) = &r.io.project_path {
            session::retire_stale_autosave(dir);
        }
    }
    if !autosave && r.io.project_path.is_some() {
        if let Some(scratch) = super::autosave_scratch_dir(r) {
            remove_scratch_dir(&scratch);
        }
        if let Some(recovered) = r.io.recovered_scratch_dir.take() {
            remove_scratch_dir(&recovered);
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
