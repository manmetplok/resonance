//! Update handlers for the docked media browser (doc #175, todo #599):
//! filesystem navigation, per-folder filtering, favourite / recent
//! management, Files/Pool tab switching, and the audition preview
//! transport.
//!
//! All of this is **transient** UI state — it is classified
//! `UndoAction::Skip` (see `undo.rs`) so nothing here lands on the undo
//! stack or in the project file, exactly like the collapse toggles. The
//! two durable things it touches are user-level, not project-level:
//!
//! * **Favourites / recent folders** live on [`crate::state::pool`] and
//!   persist in `settings.json`, so toggling a favourite or visiting a
//!   folder writes those lists back via
//!   [`crate::Resonance::persist_media_browser_settings`].
//! * **Audition** drives the engine's dedicated preview transport
//!   (`AuditionFile` / `StopAudition` / `SetAuditionOptions`), which is
//!   independent of the arrangement, transport, and undo.
//!
//! Folder scans run off the UI thread via [`scan_folder`] (which uses the
//! `resonance_common` audio-folder helper) so a slow directory never
//! blocks paint; the result comes back as
//! [`BrowserMessage::ScanCompleted`] and is applied only if the user is
//! still looking at that folder.

use std::path::{Path, PathBuf};

use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{BrowserMessage, Message};
use crate::state::FolderScan;
use crate::Resonance;

pub fn handle(app: &mut Resonance, message: BrowserMessage) -> Task<Message> {
    match message {
        BrowserMessage::SelectTab(tab) => {
            app.browser.tab = tab;
        }

        BrowserMessage::OpenFolder(path) => return open_folder(app, path),

        BrowserMessage::ScanCompleted { folder, scan } => {
            // Drop a scan whose folder the user has already navigated away
            // from — a newer scan for the current folder is (or will be)
            // in flight and owns the `scanning` flag.
            if app.browser.current_folder.as_deref() == Some(folder.as_path()) {
                app.browser.scan = scan;
                app.browser.scanning = false;
            }
        }

        BrowserMessage::SetFilter(text) => {
            app.browser.filter = text;
        }

        BrowserMessage::ToggleFavourite(path) => {
            app.pool.toggle_favourite(path);
            app.persist_media_browser_settings();
        }

        BrowserMessage::Select(target) => return select(app, target),

        BrowserMessage::Play(path) => return start_preview(app, path, 0),

        BrowserMessage::Stop => stop_preview(app),

        BrowserMessage::Scrub(frame) => {
            // Scrub = seek: restart the engine preview at the new source
            // frame for whichever row is playing (or, if none, the
            // selected row).
            if let Some(path) = app
                .browser
                .audition
                .playing
                .clone()
                .or_else(|| app.browser.audition.selected.clone())
            {
                return start_preview(app, path, frame);
            }
        }

        BrowserMessage::ToggleLoop => {
            app.browser.audition.loop_enabled = !app.browser.audition.loop_enabled;
            send_audition_options(app);
        }

        BrowserMessage::ToggleSync => {
            app.browser.audition.sync_to_tempo = !app.browser.audition.sync_to_tempo;
            send_audition_options(app);
        }

        BrowserMessage::ToggleAutoPlay => {
            app.browser.audition.auto_play = !app.browser.audition.auto_play;
        }
    }
    Task::none()
}

/// Navigate the Files tab into `path`: make it the current folder, clear
/// the per-folder filter, record it as most-recently visited (persisted
/// to user settings), and start an off-thread scan.
fn open_folder(app: &mut Resonance, path: PathBuf) -> Task<Message> {
    app.browser.current_folder = Some(path.clone());
    app.browser.filter.clear();
    // Drop the previous folder's rows so stale content doesn't flash
    // while the new scan runs.
    app.browser.scan = FolderScan::default();
    app.browser.scanning = true;

    // Visiting a folder pushes it to the recent list (user-level state).
    app.pool.push_recent_folder(path.clone());
    app.persist_media_browser_settings();

    scan_task(path)
}

/// Handle a select-to-audition. Highlights `target`; when Auto-play is on
/// a `Some` target immediately previews. A `None` target clears the
/// selection and stops any preview.
fn select(app: &mut Resonance, target: Option<PathBuf>) -> Task<Message> {
    app.browser.audition.selected = target.clone();
    match target {
        Some(path) if app.browser.audition.auto_play => start_preview(app, path, 0),
        Some(_) => Task::none(),
        None => {
            stop_preview(app);
            Task::none()
        }
    }
}

/// Start (or restart, for a scrub) the engine audition preview of `path`
/// at `start_frame`, and mirror the playing row + reset the scrub
/// playhead. Also marks `path` as the selected row so the two stay in
/// sync when playback is started directly from a row's play button.
fn start_preview(app: &mut Resonance, path: PathBuf, start_frame: u64) -> Task<Message> {
    app.browser.audition.selected = Some(path.clone());
    app.browser.audition.playing = Some(path.clone());
    app.browser.audition.position_frame = start_frame;
    let _ = app.engine.send(AudioCommand::AuditionFile { path, start_frame });
    Task::none()
}

/// Stop the current preview (if any) and reset the playing row + scrub
/// playhead. A no-op when nothing is playing, mirroring the engine's
/// silent no-op on an idle `StopAudition`.
fn stop_preview(app: &mut Resonance) {
    if app.browser.audition.playing.take().is_some() {
        app.browser.audition.position_frame = 0;
        let _ = app.engine.send(AudioCommand::StopAudition);
    }
}

/// Push the current loop / sync-to-tempo options to the engine. The engine
/// persists them across `AuditionFile` commands and applies them
/// immediately to any preview already playing.
fn send_audition_options(app: &mut Resonance) {
    let _ = app.engine.send(AudioCommand::SetAuditionOptions {
        loop_enabled: app.browser.audition.loop_enabled,
        sync_to_tempo: app.browser.audition.sync_to_tempo,
    });
}

/// Build the off-thread scan task for `folder`. The scan runs on a
/// blocking pool so a large / slow directory never stalls the UI thread;
/// the result comes back as [`BrowserMessage::ScanCompleted`] tagged with
/// the folder it scanned so a stale result can be dropped.
fn scan_task(folder: PathBuf) -> Task<Message> {
    let scan_dir = folder.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || scan_folder(&scan_dir))
                .await
                .unwrap_or_default()
        },
        move |scan| {
            Message::Browser(BrowserMessage::ScanCompleted {
                folder: folder.clone(),
                scan,
            })
        },
    )
}

/// Scan one folder for its immediate subfolders and probed audio files.
/// The audio rows come from the shared `resonance_common` helper
/// ([`resonance_common::scan_audio_folder`], todo #594); subfolders are
/// listed here since the browser needs them for navigation. Both lists are
/// sorted by path. Pure and blocking — call it off the UI thread.
pub fn scan_folder(dir: &Path) -> FolderScan {
    let files = resonance_common::scan_audio_folder(dir);
    let mut folders = list_subdirs(dir);
    folders.sort();
    FolderScan { folders, files }
}

/// List the immediate child directories of `dir` (absolute paths,
/// unsorted). An unreadable directory yields an empty list, matching the
/// audio-folder scan's behaviour.
fn list_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .collect()
}
