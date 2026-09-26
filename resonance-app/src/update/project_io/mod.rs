//! Project save / load — message dispatch + async save-collector kickoff.
//! Pure serialization lives in `serialize.rs`, the `LoadedProject` →
//! engine + state replay lives in `replay.rs`, and rfd file-dialog
//! tasks live in `dialogs.rs`.

mod dialogs;
mod instantiate;
mod replay;
pub mod replay_diff;
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
pub use replay::{migrate_auto_name, sort_plugins_by_saved_order};
pub(crate) use replay::{
    restore_drum_patterns, restore_performance, restore_pool, restore_quantize,
    restore_references,
};
pub use replay_diff::try_diff_replay;
pub use serialize::{build_project_file, plugin_states_for_save};
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
            // An offline control measurement holds the offline renderer
            // exclusively (`OfflineRenderGuard::try_acquire_exclusive`);
            // starting a WAV bounce on top of it would drive the same
            // live plugin instances from two renderers at once. Refuse,
            // mirroring how `meter.measure` refuses while a bounce runs.
            // (The control `render.mixdown` path refuses in its own
            // busy_guard before reaching this message; this covers the
            // bounce dialog.)
            if r.offline_measure_in_progress() {
                r.error_message =
                    Some("A measurement is in progress; bounce again when it finishes".into());
            } else {
                r.io.bouncing = true;
                let _ = r.engine.send(AudioCommand::BounceToWav { path });
            }
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
            // Case-insensitive: `Song.RPROJ` / `Song.Rproj` already carry
            // the extension and must not be doubled into
            // `Song.RPROJ.rproj`. Callers (the control `project.save` /
            // `save_as` handlers included) may already have normalized
            // this, but the check is repeated here so this handler stays
            // correct standalone for its other caller, the rfd save
            // dialog.
            let has_rproj_ext = std::path::Path::new(&path)
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rproj"));
            let path = if has_rproj_ext {
                std::path::PathBuf::from(path)
            } else {
                std::path::PathBuf::from(format!("{path}.rproj"))
            };
            r.io.project_path = Some(path);
            return start_save(r);
        }
        ProjectIoMessage::SavePathSelected(None) => {}
        ProjectIoMessage::OpenProject => {
            if r.refuse_project_switch_during_render() {
                return Task::none();
            }
            return dialogs::open_project_dialog();
        }
        ProjectIoMessage::OpenPathSelected(Some(path)) => {
            // The dialog may have been open since before the render
            // started (or Ctrl+O bypasses every modal), so check again
            // at the moment the project would actually be swapped.
            if r.refuse_project_switch_during_render() {
                return Task::none();
            }
            // Path and engine dir are only repointed once the load
            // succeeds (`ProjectLoaded(Ok)`): a failed open must leave
            // the still-open project tied to its own folder.
            let path = std::path::PathBuf::from(path);
            r.io.pending_open_path = Some(path.clone());
            return dialogs::load_project_task(path);
        }
        ProjectIoMessage::OpenPathSelected(None) => {}
        ProjectIoMessage::OpenRecent(path) => {
            if r.refuse_project_switch_during_render() {
                return Task::none();
            }
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
            r.io.pending_open_path = Some(path.clone());
            return dialogs::load_project_task(path);
        }
        ProjectIoMessage::ProjectSaved(Ok(()), autosave) => {
            r.io.save_state = None;
            r.io.saving = false;
            // Resolve a control-initiated save job (doc #265, todo
            // #1149) — manual saves only: an autosave completing must
            // never satisfy a client's project.save. No-op when no
            // control job carries the token.
            if !autosave {
                let path = r.io.project_path.as_ref().map(|p| p.display().to_string());
                r.control.jobs.complete_token(
                    &crate::control_jobs::JobToken::ProjectSave,
                    serde_json::json!({ "path": path, "revision": r.revision() }),
                );
            }
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
            if !autosave {
                r.control.jobs.fail_token(
                    &crate::control_jobs::JobToken::ProjectSave,
                    e.clone(),
                );
            }
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
            // Adopt the opened path now that the load succeeded — before
            // `ClearAll`, since `all_cleared` restores `project_path`
            // around the replay.
            if let Some(path) = r.io.pending_open_path.take() {
                let _ = r.engine.send(AudioCommand::SetProjectDir(path.clone()));
                r.io.project_path = Some(path);
            }
            // A control-initiated load job (todo #1149) is NOT resolved
            // here: the replay only runs once the engine confirms the
            // `ClearAll` below, on a later Tick. `all_cleared` completes
            // the `ProjectLoad` token after the replay (code review
            // UPD-02).
            let _ = r.engine.send(AudioCommand::Stop);
            r.transport.playing = false;
            r.transport.recording = false;
            r.io.loading = true;
            r.io.pending_load = Some(loaded);
            r.undo.clear();
            r.plugin_state_cache.clear();
            // Both are re-seeded from the incoming file by `replay_plugins`.
            // Dropping them together keeps a previous project's blob or
            // parked parameter list from being written into this one under
            // a colliding instance id.
            r.pending_plugin_param_overrides.clear();
            r.freeze.reset();
            r.dirty = false;
            let _ = r.engine.send(AudioCommand::ClearAll);
            r.io.has_active_project = true;
            if let Some(ref path) = r.io.project_path {
                crate::recent::add(&mut r.io.recent_projects, path);
            }
        }
        ProjectIoMessage::ProjectLoaded(Err(e)) => {
            r.io.pending_open_path = None;
            r.control.jobs.fail_token(
                &crate::control_jobs::JobToken::ProjectLoad,
                e.clone(),
            );
            r.error_message = Some(format!("Load failed: {e}"));
        }
        ProjectIoMessage::TemplateLoaded(Ok(loaded)) => {
            instantiate::begin_instantiate(r, loaded);
        }
        ProjectIoMessage::TemplateLoaded(Err(e)) => {
            // Fail a control-initiated `project.new` from a user template
            // (todo #1151). No-op when no control job carries the token.
            r.control
                .jobs
                .fail_token(&crate::control_jobs::JobToken::ProjectNew, e.clone());
            r.error_message = Some(format!("Open template failed: {e}"));
        }
        ProjectIoMessage::ExportChordSheet => {
            let pdf_bytes =
                crate::chord_sheet_pdf::build_chord_sheet_pdf(&r.compose, chord_sheet_header(r));
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

/// Tempo and meter for the exported chord sheet's page header.
///
/// Reads the song's own global tracks, NOT `r.transport` (ba todo
/// #1390): the transport's `bpm` / `time_sig_num` are playhead
/// readings, so on a song that changes meter the header used to print
/// whatever sat under the cursor — and moving the cursor changed the
/// PDF. Same trap epic #205 documented for `song.summary`.
pub fn chord_sheet_header(r: &Resonance) -> crate::chord_sheet_pdf::SongHeader {
    crate::chord_sheet_pdf::SongHeader::from_song(&r.tempo_events, &r.signature_events)
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

/// Capture the open project as a user template (todo #666), into the
/// user templates directory.
fn save_current_as_template(
    r: &Resonance,
    name: &str,
    description: &str,
    options: templates::TemplateCaptureOptions,
) -> Result<std::path::PathBuf, String> {
    let root = templates::ensure_templates_dir()
        .ok_or_else(|| "could not resolve the templates directory".to_string())?;
    save_current_as_template_in(r, &root, name, description, options)
}

/// The body of [`save_current_as_template`], with the templates root
/// passed in so a test can capture into a temp dir instead of the user's
/// real template library.
///
/// Synchronous: it serializes the current app state via
/// [`build_project_file`], pairs it with the plugin-state blobs and the
/// in-memory MIDI clips, and writes a fresh template folder via
/// [`templates::write_template`]. The plugin states come from
/// [`plugin_states_for_save`] — the app-side cache, the same one snapshot
/// undo and "Save as preset" read — rather than a fresh engine round-trip,
/// so no async save collector is needed. Passing no engine states means
/// *every* slot falls back to that cache, which is what carries a missing
/// plugin's opaque blob into the template instead of dropping it (ba doc
/// #275, P5). Returns the created folder path.
pub(crate) fn save_current_as_template_in(
    r: &Resonance,
    root: &std::path::Path,
    name: &str,
    description: &str,
    options: templates::TemplateCaptureOptions,
) -> Result<std::path::PathBuf, String> {
    let project = build_project_file(r);

    let plugin_states: Vec<(PluginInstanceId, Vec<u8>)> = plugin_states_for_save(r, Vec::new());

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
        root,
        name,
        description,
        project,
        &plugin_states,
        &midi_clips,
        options,
        created_secs,
    )
}
