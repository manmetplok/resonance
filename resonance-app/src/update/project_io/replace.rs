//! Save As onto a different existing project (BUG-62).
//!
//! A save writes `project.json`, the MIDI files and plugin blobs it names,
//! and the clip WAVs the engine holds — and leaves everything else in the
//! folder alone. Saving over *another* project therefore kept its
//! `backups/` (which the clip GC reads as references, keeping its clips
//! alive), its autosave (offered for recovery on the next open) and its
//! old media. So replacing a project is explicit: the GUI asks first
//! (`ModalState::confirm_replace_project`), the control API wants
//! `confirm: true`, and on that answer the target's history and media are
//! moved out of the way before the save starts.
//!
//! Data safety: only the entries in [`REPLACED_ENTRIES`] are touched, by
//! renaming them into one sibling trash dir (a cheap, same-filesystem
//! rename on the update loop) that a background task then deletes. Nothing
//! is moved when an entry is, holds or sits inside a directory the open
//! session still reads from (the open project, the bundle a Save As is
//! carrying clips from, the untitled scratch dir), or when another running
//! Resonance holds the target's session marker. A rename that fails puts
//! back what was already moved.

use std::path::{Path, PathBuf};

use iced::Task;

use crate::message::Message;
use crate::project::session;
use crate::project::{same_dir, PROJECT_JSON};
use crate::Resonance;

/// What a project bundle holds that belongs to the project in it: its
/// metadata, its history (backups, autosave) and its media. Replacing the
/// project clears exactly these; anything else in the folder stays.
pub(crate) const REPLACED_ENTRIES: &[&str] = &[
    PROJECT_JSON,
    crate::project::AUTOSAVE_JSON,
    crate::project::io::AUTOSAVE_SIDECAR_DIR,
    "backups",
    "audio",
    "midi",
    "plugins",
];

/// Whether a save to `target` would write over a project other than the
/// open one: there is a `project.json` there, and the folder is not the
/// open project's (nor, for an untitled project, its scratch dir).
pub(crate) fn replaces_another_project(r: &Resonance, target: &Path) -> bool {
    target.join(PROJECT_JSON).is_file()
        && !r.undo_anchor_dir().is_some_and(|open| same_dir(open, target))
}

/// The directories the session still reads from, which a replace must
/// never clear: the open project (or untitled anchor), the bundle a Save
/// As is carrying clip WAVs from, and this session's scratch dir.
fn protected_dirs(r: &Resonance) -> Vec<PathBuf> {
    [
        r.io.project_path.clone(),
        r.io.untitled_anchor.clone(),
        r.io.carry_clip_wavs_from.clone(),
        super::autosave_scratch_dir(r),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// `inner` is `outer` or inside it, once both are resolved.
fn is_within(inner: &Path, outer: &Path) -> bool {
    let resolve = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    resolve(inner).starts_with(resolve(outer))
}

/// Clear the project at `target` for a confirmed replace: check it is safe,
/// then move its [`REPLACED_ENTRIES`] into a sibling trash dir. Returns the
/// trash dir to delete (`None` when there was nothing to move), or why the
/// project was left untouched.
fn clear_replaced_project(
    target: &Path,
    protected: &[PathBuf],
    own_session_id: &str,
) -> Result<Option<PathBuf>, String> {
    if let Some(marker) = session::read_marker(target) {
        if session::marker_is_live(&marker, own_session_id) {
            return Err(format!(
                "{} is open in another Resonance window",
                target.display()
            ));
        }
    }
    let entries: Vec<PathBuf> = REPLACED_ENTRIES
        .iter()
        .map(|name| target.join(name))
        .filter(|p| p.symlink_metadata().is_ok())
        .collect();
    // A protected dir that is an entry or lies inside one would go with
    // it; an entry inside a protected dir is part of what the session still
    // reads (the target *is* the bundle a carry copies from, or holds it).
    // Resolved through symlinks, so a symlinked `audio/` pointing into the
    // open project is refused too, though the rename and the delete would
    // only take the link: when in doubt, keep the files.
    for entry in &entries {
        if let Some(dir) = protected
            .iter()
            .find(|dir| is_within(dir, entry) || is_within(entry, dir))
        {
            return Err(format!(
                "{} holds files the open project still uses ({})",
                entry.display(),
                dir.display()
            ));
        }
    }
    if entries.is_empty() {
        return Ok(None);
    }

    let parent = target
        .parent()
        .ok_or_else(|| format!("{} has no parent folder", target.display()))?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let trash = parent.join(format!(".{name}.replaced-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&trash).map_err(|e| format!("create {}: {e}", trash.display()))?;

    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for entry in entries {
        let to = trash.join(entry.file_name().expect("a joined entry name"));
        if let Err(e) = std::fs::rename(&entry, &to) {
            // Put back what already moved: the project stays whole.
            for (from, to) in moved.iter().rev() {
                if let Err(e) = std::fs::rename(to, from) {
                    tracing::warn!("restore {} from {}: {e}", from.display(), to.display());
                }
            }
            let _ = std::fs::remove_dir(&trash);
            return Err(format!("move {} aside: {e}", entry.display()));
        }
        moved.push((entry, to));
    }
    Ok(Some(trash))
}

/// Delete a replaced project's trash dir off the update loop. Best effort:
/// a dir left behind is only wasted space beside the project.
fn delete_trash(trash: PathBuf) -> Task<Message> {
    Task::future(async move {
        if let Err(e) = std::fs::remove_dir_all(&trash) {
            tracing::warn!("delete replaced project files {}: {e}", trash.display());
        }
    })
    .discard()
}

/// The Save As dialog picked `path`. Another project lives there:
/// replacing it deletes its history and media, so ask first.
pub(super) fn selected(r: &mut Resonance, path: String) -> Task<Message> {
    let path = super::save_as::with_rproj_extension(path);
    if replaces_another_project(r, &path) {
        r.modals.confirm_replace_project = Some(path);
        return Task::none();
    }
    super::save_as::save_as_to(r, path)
}

/// The answer to "Replace project X?". Cancel saves nothing and, like
/// cancelling the Save As dialog, cancels the project switch that waited
/// on the save: the work stays open.
pub(super) fn answer(r: &mut Resonance, replace: bool) -> Task<Message> {
    let Some(path) = r.modals.confirm_replace_project.take() else {
        return Task::none();
    };
    if replace {
        return save_as_confirmed(r, path);
    }
    r.modals.switch_after_save = None;
    Task::none()
}

/// The control `project.save_as` / `project.save` with `confirm: true`: a
/// client cannot answer the dialog, and `confirm` is its answer.
pub(super) fn confirmed(r: &mut Resonance, path: String) -> Task<Message> {
    save_as_confirmed(r, super::save_as::with_rproj_extension(path))
}

/// Save As to `target` with any replacement it implies already confirmed
/// (the dialog's Replace, or `project.save_as` with `confirm: true`): clear
/// the project there when it is another one, then save. A replace that is
/// not safe saves nothing and says why.
fn save_as_confirmed(r: &mut Resonance, target: PathBuf) -> Task<Message> {
    let mut cleanup = Task::none();
    if replaces_another_project(r, &target) {
        match clear_replaced_project(&target, &protected_dirs(r), r.session_id()) {
            Ok(Some(trash)) => {
                tracing::info!("[save] replacing the project at {}", target.display());
                cleanup = delete_trash(trash);
            }
            Ok(None) => {}
            Err(reason) => {
                r.banners.error_message =
                    Some(format!("Could not replace {}: {reason}", project_name(&target)));
                // The save the switch waited on never started.
                r.modals.switch_after_save = None;
                return Task::none();
            }
        }
    }
    Task::batch([cleanup, super::save_as::save_as_to(r, target)])
}

/// The name the dialog and banners call a project bundle by.
fn project_name(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
