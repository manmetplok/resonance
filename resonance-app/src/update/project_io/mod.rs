//! Project save / load — message dispatch + async save-collector kickoff.
//! Pure serialization lives in `serialize.rs`, the `LoadedProject` →
//! engine + state replay lives in `replay/` and `replay_diff.rs` (the
//! domains both share run through `reconcile/`), and rfd file-dialog
//! tasks live in `dialogs.rs`.

mod autosave;
mod dialogs;
mod instantiate;
pub mod reconcile;
pub(crate) mod recovery;
pub(crate) mod replay;
pub mod replay_diff;
mod serialize;
mod templates;

use std::collections::HashMap;

use iced::Task;
use resonance_audio::types::*;

use crate::message::*;
use crate::project::{LoadedProject, SaveCollector};
use crate::state::ProjectSwitch;
use crate::Resonance;

pub use autosave::{should_autosave, tick_autosave, AutosaveGate};
pub use dialogs::save_project_as_dialog;
pub use instantiate::{begin_instantiate, instantiate_builtin, load_user_template_task};
pub use replay::replay_loaded_project;
pub use replay::{migrate_auto_name, sort_plugins_by_saved_order};
pub use serialize::{build_project_file, plugin_states_for_save};
pub(crate) use serialize::add_session_plugin_params;
pub use templates::{
    builtin_templates, compute_summary, ensure_templates_dir, scan_templates_in,
    scan_user_templates, templates_dir, write_template, BuiltinProject, BuiltinTemplateId,
    StaleReason, StaleTemplate, Template, TemplateCaptureOptions, TemplateEntry, TemplateKind,
    TemplateMetadata, TemplateSummary,
};

#[derive(Debug, Clone)]
pub enum ProjectIoMessage {
    BounceToWav,
    BouncePathSelected(Option<String>),
    /// Cancel button of the WAV mixdown progress modal (FU-F1c): stops the
    /// in-flight render cooperatively via `AudioCommand::CancelBounce`.
    CancelBounce,
    SaveProject,
    SaveProjectAs,
    /// Begin a periodic autosave snapshot. Routed through the same async
    /// engine save state machine as [`Self::SaveProject`], but writes the
    /// metadata to `project.autosave.json`, leaves the project dirty, and
    /// targets a per-session scratch dir when the project was never saved.
    /// Fired by the change-gated autosave timer (todo #465).
    Autosave,
    /// Capture the open project as a reusable user template (todo #666).
    /// `name`/`description` label it in the picker; the two booleans are
    /// the capture toggles (carry the tempo map / the master FX chain).
    SaveAsTemplate {
        name: String,
        description: String,
        include_markers_and_tempo: bool,
        include_master_chain: bool,
    },
    OpenProject,
    /// User clicked a recent entry in the startup modal.
    OpenRecent(std::path::PathBuf),
    SavePathSelected(Option<String>),
    OpenPathSelected(Option<String>),
    /// Async save completion. The `bool` is `true` when the completed
    /// save was an autosave (routes to `last_autosave_at`, keeps `dirty`
    /// set, skips the recents list) rather than a manual save.
    ProjectSaved(Result<(), String>, bool),
    ProjectLoaded(Result<Box<LoadedProject>, String>),
    /// A disk open's async load finished, tagged with the
    /// `io.open_token` it was started under. Forwarded as
    /// [`Self::ProjectLoaded`] when the token is still current; dropped
    /// when a later open overtook it (FU-A1a).
    OpenLoadFinished(u64, Result<Box<LoadedProject>, String>),
    /// A *user* template finished loading from disk (todo #665). Carries the
    /// same `LoadedProject` payload as [`Self::ProjectLoaded`], but the
    /// instantiate handler replays it as a fresh, untitled project (path left
    /// `None`) so the template source on disk is never overwritten.
    TemplateLoaded(Result<Box<LoadedProject>, String>),
    ExportChordSheet,
    ChordSheetPathSelected(Option<String>, Vec<u8>),
    /// The user's answer to the autosave-recovery prompt (code review
    /// FU-M12a).
    RecoveryChoice(RecoveryChoice),
    /// Open `path` without the recovery prompt; `recover` loads its
    /// autosave (when one is recoverable) instead of `project.json`. The
    /// control `project.open` path: a client can't answer a modal.
    OpenResolved {
        path: std::path::PathBuf,
        recover: bool,
    },
    /// The user's answer to the unsaved-changes dialog a GUI Open / New
    /// raised (code review UX-01).
    SwitchChoice(SwitchChoice),
    /// User clicked "New Project" in the startup modal.
    StartNewProject,
    /// Replace the open project with a fresh, untitled empty one (the New
    /// Project command). Over unsaved changes it asks first (the
    /// Save / Don't save / Cancel dialog, code review UX-01); refused while
    /// a load or save is running, or an offline render owns the engine.
    NewEmptyProject,
    /// User confirmed "Save & Quit" in the unsaved-changes dialog.
    ConfirmSaveAndQuit,
    /// User confirmed "Discard & Quit" in the unsaved-changes dialog.
    ConfirmDiscardAndQuit,
    /// User cancelled the unsaved-changes quit dialog.
    CancelQuit,
    /// Switch periodic autosave on/off (persisted in settings, code review
    /// FU-M12a / ba todo #471).
    ToggleAutosave,
    /// Set the autosave interval in seconds (persisted in settings).
    SetAutosaveInterval(u32),
}

impl ProjectIoMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // Save / open / bounce / template flows: project I/O, not an edit.
            // Opening a project replaces the history wholesale instead.
            Self::BounceToWav
            | Self::BouncePathSelected(..)
            | Self::CancelBounce
            | Self::SaveProject
            | Self::SaveProjectAs
            | Self::Autosave
            | Self::SaveAsTemplate { .. }
            | Self::OpenProject
            | Self::OpenRecent(..)
            | Self::SavePathSelected(..)
            | Self::OpenPathSelected(..)
            | Self::ProjectSaved(..)
            | Self::ProjectLoaded(..)
            | Self::OpenLoadFinished(..)
            | Self::TemplateLoaded(..)
            | Self::ExportChordSheet
            | Self::ChordSheetPathSelected(..)
            | Self::RecoveryChoice(..)
            | Self::OpenResolved { .. }
            | Self::SwitchChoice(..)
            // The project lifecycle, the quit dialog and the autosave
            // settings (moved here from `UiMessage`, ARCH2-12): never a
            // project edit either.
            | Self::StartNewProject
            | Self::NewEmptyProject
            | Self::ConfirmSaveAndQuit
            | Self::ConfirmDiscardAndQuit
            | Self::CancelQuit
            | Self::ToggleAutosave
            | Self::SetAutosaveInterval(..) => UndoAction::Skip,
        }
    }
}

/// Route a `ProjectIoMessage` to the appropriate handler.
pub fn handle(r: &mut Resonance, m: ProjectIoMessage) -> Task<Message> {
    match m {
        ProjectIoMessage::StartNewProject => {
            if r.refuse_project_switch_during_render() {
                return Task::none();
            }
            return dialogs::save_project_as_dialog();
        }
        ProjectIoMessage::NewEmptyProject => {
            if r.io.loading || r.io.saving || r.io.save_state.is_some() {
                return Task::none();
            }
            if r.refuse_project_switch_during_render() {
                return Task::none();
            }
            // Over unsaved changes this asks Save / Don't save / Cancel
            // rather than refusing (code review UX-01, UX-04).
            return request_switch(r, ProjectSwitch::NewEmpty);
        }
        ProjectIoMessage::ConfirmSaveAndQuit => {
            let window_id = r.modals.confirm_quit.take();
            r.modals.quit_after_save = window_id;
            return r.update(Message::ProjectIo(ProjectIoMessage::SaveProject));
        }
        ProjectIoMessage::ConfirmDiscardAndQuit => {
            if let Some(id) = r.modals.confirm_quit.take() {
                recovery::close_session(r);
                r.engine.shutdown(std::time::Duration::from_millis(150));
                return iced::window::close(id);
            }
        }
        ProjectIoMessage::CancelQuit => {
            r.modals.confirm_quit = None;
        }
        ProjectIoMessage::ToggleAutosave => {
            let enabled = &mut r.settings.autosave.enabled;
            *enabled = !*enabled;
            crate::settings::persist(&r.settings);
        }
        ProjectIoMessage::SetAutosaveInterval(secs) => {
            // The trigger reads the setting on every tick, so the new
            // spacing applies from the next one.
            r.settings.autosave.interval_secs = secs.max(1);
            crate::settings::persist(&r.settings);
        }
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
                r.banners.error_message =
                    Some("A measurement is in progress; bounce again when it finishes".into());
            } else {
                r.io.bouncing = true;
                r.io.bounce_fraction = 0.0;
                r.io.bounce_cancel_requested = false;
                r.io.bounce_target = std::path::Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone());
                let _ = r.engine.send(AudioCommand::BounceToWav { path });
            }
        }
        ProjectIoMessage::BouncePathSelected(None) => {}
        ProjectIoMessage::CancelBounce => {
            if r.io.bouncing && !r.io.bounce_cancel_requested {
                r.io.bounce_cancel_requested = true;
                let _ = r.engine.send(AudioCommand::CancelBounce);
            }
        }
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
                r.banners.error_message = Some(format!("Save template failed: {e}"));
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
            // The session writes clip WAVs into this bundle from now on;
            // one it already holds must not get its clip ids re-issued
            // (FU-A6c), nor its asset ids (D-7a).
            r.seed_clip_ids_on_disk(&path);
            r.seed_asset_ids_on_disk(&path);
            r.io.project_path = Some(path);
            return start_save(r);
        }
        ProjectIoMessage::SavePathSelected(None) => {
            // "Save" on the unsaved-changes dialog of an untitled project
            // went through Save As; cancelling that dialog cancels the
            // switch too — the work stays open, nothing is lost.
            r.modals.switch_after_save = None;
        }
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
            return request_switch(r, ProjectSwitch::Open(path));
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
                r.banners.error_message = Some(format!(
                    "Project not found: {} — removed from recent projects.",
                    path.display()
                ));
                crate::recent::remove(&mut r.io.recent_projects, &path);
                return Task::none();
            }
            return request_switch(r, ProjectSwitch::Open(path));
        }
        ProjectIoMessage::SwitchChoice(choice) => {
            return answer_switch(r, choice);
        }
        ProjectIoMessage::ProjectSaved(Ok(()), autosave) => {
            finish_save_write(r);
            recovery::after_save(r, autosave);
            // Any completed write proves the location takes writes again:
            // a manual save of a titled project lands in the very bundle
            // the autosave targets (code review UX-13).
            r.banners.clear_autosave_failures();
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
                // Clean only what the save captured: an edit made while
                // the files were being written bumped the revision and is
                // not on disk (code review STATE-09). A completion with no
                // recorded capture keeps the old unconditional clean.
                let captured = r.io.save_capture_revision.take();
                if captured.is_none_or(|rev| rev == r.revision()) {
                    r.session.dirty = false;
                }
                r.io.has_active_project = true;
                r.io.last_saved_at = Some(std::time::SystemTime::now());
                reap_orphaned_vocal_takes(r);
                if let Some(ref path) = r.io.project_path {
                    crate::recent::add(&mut r.io.recent_projects, path);
                }
                if let Some(switch) = r.modals.switch_after_save.take() {
                    // Still dirty: the save missed a late edit. Ask again
                    // rather than switch over it.
                    if r.session.dirty {
                        r.modals.confirm_switch = Some(switch);
                        return Task::none();
                    }
                    return perform_switch(r, switch);
                }
                if let Some(id) = r.modals.quit_after_save.take() {
                    // Still dirty: the save missed a late edit. Ask again
                    // rather than close over it.
                    if r.session.dirty {
                        r.modals.confirm_quit = Some(id);
                        return Task::none();
                    }
                    recovery::close_session(r);
                    r.engine.shutdown(std::time::Duration::from_millis(150));
                    return iced::window::close(id);
                }
            }
        }
        ProjectIoMessage::ProjectSaved(Err(e), autosave) => {
            finish_save_write(r);
            if !autosave {
                r.io.save_capture_revision = None;
                // The save the switch waited on failed: stay put, with the
                // error showing, rather than switch over unsaved work.
                r.modals.switch_after_save = None;
            }
            if !autosave {
                r.control.jobs.fail_token(
                    &crate::control_jobs::JobToken::ProjectSave,
                    e.clone(),
                );
            }
            if autosave {
                // A failed autosave must never interrupt the user with a
                // modal — the timer will try again. One miss is only
                // logged; a run of them raises the persistent indicator
                // (code review UX-13).
                note_autosave_failure(r, e);
            } else {
                r.modals.quit_after_save = None;
                r.banners.error_message = Some(format!("Save failed: {e}"));
            }
        }
        ProjectIoMessage::OpenLoadFinished(token, result) => {
            // A later open overtook this one: its result is stale, and
            // adopting it would pair this content with the later open's
            // path (or, on failure, clear that open's pending slot).
            if token != r.io.open_token || r.io.pending_open_path.is_none() {
                tracing::debug!("dropping the result of a superseded project open");
                return Task::none();
            }
            return handle(r, ProjectIoMessage::ProjectLoaded(result));
        }
        ProjectIoMessage::ProjectLoaded(Ok(loaded)) => {
            // Adopt the opened path now that the load succeeded — before
            // `ClearAll`, since `all_cleared` restores `project_path`
            // around the replay.
            let pending = r.io.pending_open_path.take();
            if let Some(path) = pending.filter(|_| !recovery::loads_untitled(r)) {
                let _ = r.engine.send(AudioCommand::SetProjectDir(path.clone()));
                r.io.project_path = Some(path);
            }
            recovery::sync_session_marker(r);
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
            r.session.undo.clear();
            r.plugin_mirror.state_cache.clear();
            r.plugin_mirror.owed_blobs.clear();
            r.plugin_mirror.kit_info.clear();
            r.plugin_mirror.output_ports.clear();
            // Both are re-seeded from the incoming file by the
            // `PluginState` reconcile domain. Dropping them together keeps a previous project's blob or
            // parked parameter list from being written into this one under
            // a colliding instance id.
            r.presets.pending_plugin_param_overrides.clear();
            r.freeze.reset();
            // Placements queued against the old project (code review UPD-04).
            r.media.pool_import.clear();
            r.session.dirty = false;
            let _ = r.engine.send(AudioCommand::ClearAll);
            r.io.has_active_project = true;
            if let Some(ref path) = r.io.project_path {
                crate::recent::add(&mut r.io.recent_projects, path);
            }
        }
        ProjectIoMessage::ProjectLoaded(Err(e)) => {
            r.io.pending_open_path = None;
            r.io.load_recovery = None;
            r.control.jobs.fail_token(
                &crate::control_jobs::JobToken::ProjectLoad,
                e.clone(),
            );
            r.banners.error_message = Some(format!("Load failed: {e}"));
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
            r.banners.error_message = Some(format!("Open template failed: {e}"));
        }
        ProjectIoMessage::ExportChordSheet => {
            let pdf_bytes =
                crate::chord_sheet_pdf::build_chord_sheet_pdf(&r.compose, chord_sheet_header(r));
            return dialogs::chord_sheet_dialog(pdf_bytes);
        }
        ProjectIoMessage::ChordSheetPathSelected(Some(path), data) => {
            if let Err(e) = std::fs::write(&path, &data) {
                r.banners.error_message = Some(format!("Export failed: {e}"));
            }
        }
        ProjectIoMessage::ChordSheetPathSelected(None, _) => {}
        ProjectIoMessage::RecoveryChoice(choice) => {
            return recovery::handle_choice(r, choice);
        }
        ProjectIoMessage::OpenResolved { path, recover } => {
            return recovery::open_resolved(r, path, recover);
        }
    }
    Task::none()
}

/// Start an async disk open of `path` under a fresh open token. The path
/// is only adopted when the load succeeds, and only if no later open has
/// replaced this one in the meantime (FU-A1a).
/// A GUI request to replace the open project (Open, New). Over unsaved
/// changes it parks the request behind the Save / Don't save / Cancel
/// dialog instead of discarding them (code review UX-01); otherwise it
/// switches right away. The control API's `project.open` / `project.new`
/// don't come here — they answer `needs_confirmation`.
pub(crate) fn request_switch(r: &mut Resonance, switch: ProjectSwitch) -> Task<Message> {
    if r.session.dirty && r.io.has_active_project {
        r.modals.confirm_switch = Some(switch);
        return Task::none();
    }
    perform_switch(r, switch)
}

/// The user's answer to the unsaved-changes dialog.
fn answer_switch(r: &mut Resonance, choice: SwitchChoice) -> Task<Message> {
    let Some(switch) = r.modals.confirm_switch.take() else {
        return Task::none();
    };
    match choice {
        SwitchChoice::Cancel => Task::none(),
        SwitchChoice::Discard => perform_switch(r, switch),
        SwitchChoice::Save => {
            // An untitled project's Save is a Save As: the switch waits for
            // that dialog and the save it starts (`ProjectSaved(Ok)`), and
            // is dropped if either is cancelled or fails.
            r.modals.switch_after_save = Some(switch);
            r.update(Message::ProjectIo(ProjectIoMessage::SaveProject))
        }
    }
}

/// Carry out a project switch, re-checking what may have changed since it
/// was asked for (a render started, a load or save began).
fn perform_switch(r: &mut Resonance, switch: ProjectSwitch) -> Task<Message> {
    if r.refuse_project_switch_during_render() {
        return Task::none();
    }
    match switch {
        ProjectSwitch::Open(path) => {
            if recovery::prompt_before_open(r, &path) {
                return Task::none();
            }
            start_open(r, path)
        }
        ProjectSwitch::NewEmpty => {
            if r.io.loading || r.io.saving || r.io.save_state.is_some() {
                return Task::none();
            }
            instantiate_builtin(r, BuiltinTemplateId::Empty);
            Task::none()
        }
    }
}

fn start_open(r: &mut Resonance, path: std::path::PathBuf) -> Task<Message> {
    r.io.open_token = r.io.open_token.wrapping_add(1);
    r.io.pending_open_path = Some(path.clone());
    // A plain open supersedes an overtaken recovery open's intent too.
    r.io.load_recovery = None;
    dialogs::load_project_task(path, r.io.open_token)
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
/// per-session scratch dir under `<app data>/resonance/autosave/`. The
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
    // Never run two engine round-trips at once: the engine's
    // `ClipsSavedToProjectDir` / `AllPluginStatesSaved` carry no tag, so a
    // second collector would take the first one's results — a manual Save
    // As finishing with an autosave's scratch-dir clips (code review
    // STATE-11). An autosave backs off (the timer retries); a manual save
    // is queued and starts the moment the in-flight collector completes
    // (`start_queued_save`).
    if r.io.save_state.is_some() {
        if !autosave {
            r.io.manual_save_queued = true;
            r.io.saving = true;
        }
        return Task::none();
    }

    let path = match (&r.io.project_path, autosave) {
        (Some(p), _) => p.clone(),
        // Never-saved project + autosave: snapshot into a scratch dir.
        (None, true) => match autosave_scratch_dir(r) {
            Some(p) => p,
            None => {
                note_autosave_failure(r, "no app-data directory available".to_owned());
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
            note_autosave_failure(r, format!("create dir {}: {e}", path.display()));
        } else {
            r.banners.error_message = Some(format!("Create project directory: {e}"));
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
        clips_error: None,
        started: std::time::Instant::now(),
    });
    let _ = r.engine.send(AudioCommand::SaveClipsToProjectDir);
    let _ = r.engine.send(AudioCommand::SaveAllPluginStates);
    Task::none()
}

/// Fail a save whose engine round-trip did not produce what the write
/// needs: the engine could not write a clip (`AudioEvent::ClipsSaveFailed`)
/// or never answered at all (the tick watchdog). The collector is already
/// taken; this routes through the same completion as a failed write —
/// `saving` cleared, the `ProjectSave` control job failed, the banner (or,
/// for an autosave, the failure count) — and then runs any manual save
/// queued behind it (code review STATE2-02).
pub(crate) fn fail_collected_save(
    r: &mut Resonance,
    save: SaveCollector,
    reason: String,
) -> Task<Message> {
    tracing::warn!(
        autosave = save.autosave,
        "[save] {} not collected: {reason}",
        save.path.display()
    );
    let task = handle(r, ProjectIoMessage::ProjectSaved(Err(reason), save.autosave));
    start_queued_save(r);
    task
}

/// How long a save may wait on its engine round-trip before the tick
/// watchdog abandons it (STATE2-02). Generous: a Save As copies or
/// transcodes every clip WAV on the engine thread, which can take a while
/// for a long session on a slow disk — the watchdog only has to beat
/// "forever".
pub(crate) const SAVE_COLLECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// Tick-driven safety net for STATE2-02: a collector the engine never
/// answered (a reply lost to a dead engine, or a failure path that raised
/// only `AudioEvent::Error`) is failed instead of wedging every later
/// save, open and new.
pub(crate) fn check_save_watchdog(r: &mut Resonance) -> Option<Task<Message>> {
    let stale = r
        .io
        .save_state
        .as_ref()
        .is_some_and(|s| s.started.elapsed() >= SAVE_COLLECT_TIMEOUT);
    if !stale {
        return None;
    }
    let save = r.io.save_state.take().expect("checked above");
    let reason = format!(
        "the audio engine did not answer within {} s",
        SAVE_COLLECT_TIMEOUT.as_secs()
    );
    Some(fail_collected_save(r, save, reason))
}

/// One more autosave miss: logged every time, shown once
/// [`crate::state::AUTOSAVE_FAILURES_BEFORE_INDICATOR`] land in a row
/// (code review UX-13).
fn note_autosave_failure(r: &mut Resonance, reason: String) {
    tracing::warn!("Autosave failed: {reason}");
    r.banners.note_autosave_failure(reason);
}

/// Start the manual save that [`begin_save`] queued behind an in-flight
/// collector, now that the collector has taken its engine results.
pub(crate) fn start_queued_save(r: &mut Resonance) {
    if r.io.save_state.is_none() && std::mem::take(&mut r.io.manual_save_queued) {
        let _ = begin_save(r, false);
    }
}

/// A save's async write finished (either outcome). Only drop the
/// `saving` flag when no other save is collecting or queued: a manual
/// save may have started while an autosave was writing, and the
/// autosave's completion must not wipe its collector (STATE-11).
/// After a manual save: delete rendered vocal WAVs in the project's
/// `audio/` that no installed vocal clip points at (FU-B3). Skipped while
/// a render is in flight — its WAV may already be on disk, waiting for
/// the completion to install it.
fn reap_orphaned_vocal_takes(r: &Resonance) {
    use crate::update::compose::vocal_audio_io;
    let Some(project) = r.io.project_path.as_deref() else {
        return;
    };
    if !r.compose.vocal_audio.in_flight_render.is_empty() {
        return;
    }
    let keep = r
        .compose
        .vocal_audio
        .clips
        .values()
        .map(|(_, path)| path.clone())
        .collect();
    let removed =
        vocal_audio_io::reap_orphaned_takes(&vocal_audio_io::vocal_audio_dir(Some(project)), &keep);
    if !removed.is_empty() {
        tracing::info!("[vocal] removed {} unreferenced rendered take(s)", removed.len());
    }
}

fn finish_save_write(r: &mut Resonance) {
    if r.io.save_state.is_none() && !r.io.manual_save_queued {
        r.io.saving = false;
    }
}

/// Scratch directory for autosaving a never-saved project:
/// `<app data>/resonance/autosave/<session-id>/`. The per-session id
/// keeps concurrent app instances from stomping on each other's
/// snapshots. App data rather than the cache dir: it is the only copy of
/// an untitled session's work, which a cache cleaner may delete; and
/// through [`crate::user_dirs`] a test app writes under its hermetic temp
/// root instead of the developer's real dir (code review FU-M12b).
/// `None` when the platform has no data directory.
pub(crate) fn autosave_scratch_dir(r: &Resonance) -> Option<std::path::PathBuf> {
    autosave_scratch_root().map(|root| root.join(r.session_id()))
}

/// Give an untitled project that just landed (Ctrl+N, a template, a
/// recovered untitled session) a home for its clip WAVs: this session's
/// [`autosave_scratch_dir`], created now and handed to the engine as its
/// project dir, and remembered as `io.untitled_anchor`.
///
/// Two things depend on it (code review UX-03, STATE2-04):
///
/// - **Undo.** A snapshot names a clip's audio only by
///   `audio/clip_<id>.wav`, persisted by `PersistClipWavs` into the
///   engine's project dir and reloaded from the anchor on restore, so an
///   untitled project could record no history until its first Save As.
/// - **Where recordings and imports stream.** Before this the engine kept
///   whatever dir it had: the previously open project's folder, or the
///   replay's `""` (the process cwd) for a built-in template.
///
/// The dir is the one the autosave already writes to, so the two share a
/// clip id space and files; it is removed on a clean quit, and its WAVs
/// carry over into the bundle on the first Save As ([`recovery::after_save`]).
pub(crate) fn anchor_untitled_project(r: &mut Resonance) {
    r.io.untitled_anchor = None;
    let Some(dir) = autosave_scratch_dir(r) else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("untitled project scratch dir {}: {e}", dir.display());
        return;
    }
    let _ = r.engine.send(AudioCommand::SetProjectDir(dir.clone()));
    r.io.untitled_anchor = Some(dir);
}

/// Parent of every session's [`autosave_scratch_dir`].
pub(crate) fn autosave_scratch_root() -> Option<std::path::PathBuf> {
    crate::user_dirs::data_dir().map(|d| d.join("resonance").join("autosave"))
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
        .map(|mc| (mc.id, mc.notes.as_ref().clone()))
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
