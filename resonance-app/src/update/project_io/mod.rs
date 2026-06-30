//! Project save / load — message dispatch + async save-collector kickoff.
//! Pure serialization lives in `serialize.rs`, the `LoadedProject` →
//! engine + state replay lives in `replay.rs`, and rfd file-dialog
//! tasks live in `dialogs.rs`.

mod dialogs;
mod instantiate;
mod replay;
mod replay_diff;
mod serialize;
mod templates;

use std::collections::HashMap;

use iced::Task;
use resonance_audio::types::*;

use crate::message::*;
use crate::project::SaveCollector;
use crate::Resonance;

pub use dialogs::save_project_as_dialog;
pub use instantiate::{begin_instantiate, instantiate_builtin, load_user_template_task};
pub use replay::replay_loaded_project;
pub(crate) use replay::{restore_performance, restore_pool, restore_quantize, restore_references};
pub use replay_diff::try_diff_replay;
pub use serialize::build_project_file;
pub use templates::{
    builtin_templates, compute_summary, ensure_templates_dir, scan_templates_in,
    scan_user_templates, templates_dir, write_template, BuiltinProject, BuiltinTemplateId,
    StaleReason, StaleTemplate, Template, TemplateCaptureOptions, TemplateEntry, TemplateKind,
    TemplateMetadata, TemplateSummary,
};

/// Route a `ProjectIoMessage` to the appropriate handler.
pub fn handle(r: &mut Resonance, m: ProjectIoMessage) -> Task<Message> {
    match m {
        ProjectIoMessage::BounceToWav => {
            return dialogs::bounce_dialog();
        }
        ProjectIoMessage::BouncePathSelected(Some(path)) => {
            r.io.bouncing = true;
            let _ = r.engine.send(AudioCommand::BounceToWav { path });
        }
        ProjectIoMessage::BouncePathSelected(None) => {}
        ProjectIoMessage::SaveProject => {
            if r.io.project_path.is_some() {
                return start_save(r);
            } else {
                return r.update(Message::ProjectIo(ProjectIoMessage::SaveProjectAs));
            }
        }
        ProjectIoMessage::SaveProjectAs => {
            return dialogs::save_project_as_dialog();
        }
        ProjectIoMessage::Autosave => {
            return start_autosave(r);
        }
        ProjectIoMessage::SaveAsTemplate {
            name,
            description,
            include_markers_and_tempo,
            include_master_chain,
        } => {
            let options = templates::TemplateCaptureOptions {
                include_markers_and_tempo,
                include_master_chain,
            };
            if let Err(e) = save_current_as_template(r, &name, &description, options) {
                r.error_message = Some(format!("Save template failed: {e}"));
            }
        }
        ProjectIoMessage::SavePathSelected(Some(path)) => {
            let path = if path.ends_with(".rproj") {
                std::path::PathBuf::from(path)
            } else {
                std::path::PathBuf::from(format!("{path}.rproj"))
            };
            r.io.project_path = Some(path);
            return start_save(r);
        }
        ProjectIoMessage::SavePathSelected(None) => {}
        ProjectIoMessage::OpenProject => {
            return dialogs::open_project_dialog();
        }
        ProjectIoMessage::OpenPathSelected(Some(path)) => {
            let path = std::path::PathBuf::from(path);
            r.io.project_path = Some(path.clone());
            let _ = r.engine.send(AudioCommand::SetProjectDir(path.clone()));
            return dialogs::load_project_task(path);
        }
        ProjectIoMessage::OpenPathSelected(None) => {}
        ProjectIoMessage::OpenRecent(path) => {
            // The recent list is no longer pruned with a stat-per-entry
            // sweep at startup (slow on NFS / removable media), so a
            // clicked entry may point at a project that's been deleted
            // or whose volume isn't mounted. Check here, at the moment
            // it matters: surface the error and drop the dead entry.
            if !path.exists() {
                r.error_message = Some(format!(
                    "Project not found: {} — removed from recent projects.",
                    path.display()
                ));
                crate::recent::remove(&mut r.io.recent_projects, &path);
                return Task::none();
            }
            r.io.project_path = Some(path.clone());
            let _ = r.engine.send(AudioCommand::SetProjectDir(path.clone()));
            return dialogs::load_project_task(path);
        }
        ProjectIoMessage::ProjectSaved(Ok(()), autosave) => {
            r.io.save_state = None;
            r.io.saving = false;
            if autosave {
                // An autosave is a recovery snapshot, not a commit: it
                // must leave `dirty` set (the project still differs from
                // the last manual save), never touch the recents list,
                // and never satisfy a pending quit-after-save.
                r.io.last_autosave_at = Some(std::time::SystemTime::now());
            } else {
                r.dirty = false;
                r.io.has_active_project = true;
                r.io.last_saved_at = Some(std::time::SystemTime::now());
                if let Some(ref path) = r.io.project_path {
                    crate::recent::add(&mut r.io.recent_projects, path);
                }
                if let Some(id) = r.quit_after_save.take() {
                    r.engine.shutdown(std::time::Duration::from_millis(150));
                    return iced::window::close(id);
                }
            }
        }
        ProjectIoMessage::ProjectSaved(Err(e), autosave) => {
            r.io.save_state = None;
            r.io.saving = false;
            if autosave {
                // A failed autosave must never interrupt the user with a
                // modal — the timer will try again. Log and move on.
                eprintln!("Autosave failed: {e}");
            } else {
                r.quit_after_save = None;
                r.error_message = Some(format!("Save failed: {e}"));
            }
        }
        ProjectIoMessage::ProjectLoaded(Ok(loaded)) => {
            let _ = r.engine.send(AudioCommand::Stop);
            r.transport.playing = false;
            r.transport.recording = false;
            r.io.loading = true;
            r.io.pending_load = Some(loaded);
            r.undo.clear();
            r.plugin_state_cache.clear();
            r.freeze.reset();
            r.dirty = false;
            let _ = r.engine.send(AudioCommand::ClearAll);
            r.io.has_active_project = true;
            if let Some(ref path) = r.io.project_path {
                crate::recent::add(&mut r.io.recent_projects, path);
            }
        }
        ProjectIoMessage::ProjectLoaded(Err(e)) => {
            r.error_message = Some(format!("Load failed: {e}"));
        }
        ProjectIoMessage::TemplateLoaded(Ok(loaded)) => {
            instantiate::begin_instantiate(r, loaded);
        }
        ProjectIoMessage::TemplateLoaded(Err(e)) => {
            r.error_message = Some(format!("Open template failed: {e}"));
        }
        ProjectIoMessage::ExportChordSheet => {
            let pdf_bytes = crate::chord_sheet_pdf::build_chord_sheet_pdf(
                &r.compose,
                r.transport.bpm,
                r.transport.time_sig_num,
            );
            return dialogs::chord_sheet_dialog(pdf_bytes);
        }
        ProjectIoMessage::ChordSheetPathSelected(Some(path), data) => {
            if let Err(e) = std::fs::write(&path, &data) {
                r.error_message = Some(format!("Export failed: {e}"));
            }
        }
        ProjectIoMessage::ChordSheetPathSelected(None, _) => {}
    }
    Task::none()
}

/// Begin an async manual save. Requires `r.io.project_path` to already be
/// set; callers use `Message::ProjectIo(ProjectIoMessage::SaveProjectAs)`
/// first if the project has never been saved.
pub fn start_save(r: &mut Resonance) -> Task<Message> {
    begin_save(r, false)
}

/// Begin an async autosave snapshot. Unlike [`start_save`] this works on
/// a never-saved project too: with no `project_path` it targets a
/// per-session scratch dir under `cache_dir()/resonance/autosave/`. The
/// snapshot routes to `project.autosave.json` and the completion handler
/// leaves the project dirty (see [`ProjectIoMessage::Autosave`]).
pub fn start_autosave(r: &mut Resonance) -> Task<Message> {
    begin_save(r, true)
}

/// Shared save kickoff. Initializes the `SaveCollector` state machine,
/// tells the engine which directory to target, and fires the two
/// parallel engine requests (clip files + plugin states). The `autosave`
/// flag rides along on the collector so the completion path
/// (`engine_events::project_io`) routes correctly.
fn begin_save(r: &mut Resonance, autosave: bool) -> Task<Message> {
    // Never run two saves at once: a second collector would clobber the
    // first. A manual save the user explicitly triggered wins, so only
    // an autosave backs off here — the timer will retry next tick.
    if autosave && r.io.save_state.is_some() {
        return Task::none();
    }

    let path = match (&r.io.project_path, autosave) {
        (Some(p), _) => p.clone(),
        // Never-saved project + autosave: snapshot into a scratch dir.
        (None, true) => match autosave_scratch_dir(r) {
            Some(p) => p,
            None => {
                eprintln!("Autosave skipped: no cache directory available.");
                return Task::none();
            }
        },
        // A manual save with no path is a programming error here — the
        // caller routes through SaveProjectAs first.
        (None, false) => return Task::none(),
    };

    // Make sure the directory exists before the engine tries to write
    // clip WAVs into `{path}/audio/`. For a brand-new project this is the
    // first time the directory is created.
    if let Err(e) = std::fs::create_dir_all(&path) {
        if autosave {
            eprintln!("Autosave skipped: create dir {}: {e}", path.display());
        } else {
            r.error_message = Some(format!("Create project directory: {e}"));
        }
        return Task::none();
    }

    r.io.saving = true;
    let _ = r.engine.send(AudioCommand::SetProjectDir(path.clone()));
    r.io.save_state = Some(SaveCollector {
        path,
        clip_files: HashMap::new(),
        plugin_states: Vec::new(),
        clips_done: false,
        plugins_done: false,
        autosave,
    });
    let _ = r.engine.send(AudioCommand::SaveClipsToProjectDir);
    let _ = r.engine.send(AudioCommand::SaveAllPluginStates);
    Task::none()
}

/// Scratch directory for autosaving a never-saved project:
/// `cache_dir()/resonance/autosave/<session-id>/`. The per-session id
/// keeps concurrent app instances from stomping on each other's
/// snapshots. `None` when the platform has no cache directory.
fn autosave_scratch_dir(r: &Resonance) -> Option<std::path::PathBuf> {
    dirs::cache_dir().map(|c| c.join("resonance").join("autosave").join(r.session_id()))
}

/// Capture the open project as a user template (todo #666).
///
/// Synchronous: it serializes the current app state via
/// [`build_project_file`], pairs it with the cached plugin-state blobs and
/// the in-memory MIDI clips, and writes a fresh template folder under the
/// user templates dir via [`templates::write_template`]. The plugin states
/// come from `plugin_state_cache` (the same snapshot undo and "Save as
/// preset" use) rather than a fresh engine round-trip, so no async save
/// collector is needed. Returns the created folder path.
fn save_current_as_template(
    r: &Resonance,
    name: &str,
    description: &str,
    options: templates::TemplateCaptureOptions,
) -> Result<std::path::PathBuf, String> {
    let root = templates::ensure_templates_dir()
        .ok_or_else(|| "could not resolve the templates directory".to_string())?;

    let project = build_project_file(r);

    let plugin_states: Vec<(PluginInstanceId, Vec<u8>)> = r
        .plugin_state_cache
        .iter()
        .map(|(id, data)| (*id, data.clone()))
        .collect();

    let midi_clips: Vec<(ClipId, Vec<MidiNote>)> = r
        .midi_clips
        .iter()
        .map(|mc| (mc.id, mc.notes.clone()))
        .collect();

    let created_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    templates::write_template(
        &root,
        name,
        description,
        project,
        &plugin_states,
        &midi_clips,
        options,
        created_secs,
    )
}
