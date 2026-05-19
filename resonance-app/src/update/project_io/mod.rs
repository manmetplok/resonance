//! Project save / load — message dispatch + async save-collector kickoff.
//! Pure serialization lives in `serialize.rs`, the `LoadedProject` →
//! engine + state replay lives in `replay.rs`, and rfd file-dialog
//! tasks live in `dialogs.rs`.

mod dialogs;
mod replay;
mod replay_diff;
mod serialize;

use std::collections::HashMap;

use iced::Task;
use resonance_audio::types::*;

use crate::message::*;
use crate::project::SaveCollector;
use crate::Resonance;

pub use dialogs::save_project_as_dialog;
pub use replay::replay_loaded_project;
pub use replay_diff::try_diff_replay;
pub use serialize::build_project_file;

/// Route a `ProjectIoMessage` to the appropriate handler.
pub fn handle(r: &mut Resonance, m: ProjectIoMessage) -> Task<Message> {
    match m {
        ProjectIoMessage::BounceToWav => {
            return dialogs::bounce_dialog();
        }
        ProjectIoMessage::BouncePathSelected(Some(path)) => {
            r.io.bouncing = true;
            r.engine.send(AudioCommand::BounceToWav { path });
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
            r.engine.send(AudioCommand::SetProjectDir(path.clone()));
            return dialogs::load_project_task(path);
        }
        ProjectIoMessage::OpenPathSelected(None) => {}
        ProjectIoMessage::OpenRecent(path) => {
            r.io.project_path = Some(path.clone());
            r.engine.send(AudioCommand::SetProjectDir(path.clone()));
            return dialogs::load_project_task(path);
        }
        ProjectIoMessage::ProjectSaved(Ok(())) => {
            r.io.save_state = None;
            r.dirty = false;
            r.io.has_active_project = true;
            if let Some(ref path) = r.io.project_path {
                crate::recent::add(&mut r.io.recent_projects, path);
            }
            if let Some(id) = r.quit_after_save.take() {
                r.engine.shutdown(std::time::Duration::from_millis(150));
                return iced::window::close(id);
            }
        }
        ProjectIoMessage::ProjectSaved(Err(e)) => {
            r.io.save_state = None;
            r.quit_after_save = None;
            r.error_message = Some(format!("Save failed: {e}"));
        }
        ProjectIoMessage::ProjectLoaded(Ok(loaded)) => {
            r.engine.send(AudioCommand::Stop);
            r.transport.playing = false;
            r.transport.recording = false;
            r.io.loading = true;
            r.io.pending_load = Some(loaded);
            r.undo.clear();
            r.plugin_state_cache.clear();
            r.dirty = false;
            r.engine.send(AudioCommand::ClearAll);
            r.io.has_active_project = true;
            if let Some(ref path) = r.io.project_path {
                crate::recent::add(&mut r.io.recent_projects, path);
            }
        }
        ProjectIoMessage::ProjectLoaded(Err(e)) => {
            r.error_message = Some(format!("Load failed: {e}"));
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

/// Begin an async save. Requires `r.io.project_path` to already be set;
/// callers use `Message::ProjectIo(ProjectIoMessage::SaveProjectAs)`
/// first if the project has never been saved. Initializes the
/// `SaveCollector` state machine, tells the engine which directory to
/// target, and kicks off the two parallel engine requests (clip files +
/// plugin states).
pub fn start_save(r: &mut Resonance) -> Task<Message> {
    let path = match &r.io.project_path {
        Some(p) => p.clone(),
        None => return Task::none(),
    };

    // Make sure the project directory exists before the engine tries to
    // write clip WAVs into `{path}/audio/`. For a brand-new project
    // this is the first time the directory is created.
    if let Err(e) = std::fs::create_dir_all(&path) {
        r.error_message = Some(format!("Create project directory: {e}"));
        return Task::none();
    }

    r.engine.send(AudioCommand::SetProjectDir(path.clone()));
    r.io.save_state = Some(SaveCollector {
        path,
        clip_files: HashMap::new(),
        plugin_states: Vec::new(),
        clips_done: false,
        plugins_done: false,
    });
    r.engine.send(AudioCommand::SaveClipsToProjectDir);
    r.engine.send(AudioCommand::SaveAllPluginStates);
    Task::none()
}
