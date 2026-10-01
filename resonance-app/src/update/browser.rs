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

use crate::message::Message;
use crate::state::{BrowserTab, FolderScan};
use crate::Resonance;

/// Docked media-browser interaction (doc #175, todo #599): filesystem
/// navigation, filtering, favourite / recent management, tab switching,
/// and the audition preview transport. Routed through
/// `update::browser::handle`.
///
/// Every variant is **transient** — classified `UndoAction::Skip` (like
/// the collapse toggles) so none of it lands on the undo stack or in the
/// project file. The audition variants additionally drive the engine's
/// preview transport (`AuditionFile` / `StopAudition` /
/// `SetAuditionOptions`); favourite / recent changes are mirrored into
/// user settings (`settings.json`), which is user-level state, not project
/// persistence.
#[derive(Debug, Clone)]
pub enum BrowserMessage {
    // -- Panel chrome -------------------------------------------------
    /// Show / hide the docked media-browser panel in the Arrange view.
    /// Dispatched by the "Media" chrome toggle and the panel header's
    /// collapse caret. Pure transient UI state (never persisted / undone).
    ToggleVisible,

    // -- Tabs & navigation --------------------------------------------
    /// Switch between the Files and Pool tabs.
    SelectTab(BrowserTab),
    /// Navigate the Files tab into `path` (a folder row, a breadcrumb
    /// crumb, or a favourite / recent shelf entry). Sets it as the current
    /// folder, clears the per-folder filter, records it as most-recently
    /// visited, and kicks off an off-thread scan.
    OpenFolder(std::path::PathBuf),
    /// An off-thread folder scan finished. Applied only when `folder`
    /// still matches the current folder (a scan for a folder the user has
    /// since left is dropped). Clears the `scanning` flag.
    ScanCompleted {
        folder: std::path::PathBuf,
        scan: FolderScan,
    },
    /// Set the current folder's case-insensitive file-name filter.
    SetFilter(String),

    // -- Favourites / recent ------------------------------------------
    /// Toggle whether `path` is a pinned favourite folder, persisting the
    /// updated favourites list to user settings.
    ToggleFavourite(std::path::PathBuf),

    // -- Audition preview ---------------------------------------------
    /// Select `path` as the row to audition. Highlights it; when Auto-play
    /// is on, immediately starts previewing it. `None` clears the
    /// selection (and stops any preview started from it).
    Select(Option<std::path::PathBuf>),
    /// Start previewing `path` from its start through the engine.
    Play(std::path::PathBuf),
    /// Stop the current audition preview.
    Stop,
    /// Scrub the current preview to `frame` (seek): restarts the engine
    /// preview of the playing / selected row at that source frame.
    Scrub(u64),
    /// Toggle looping of the preview, pushing the new options to the
    /// engine.
    ToggleLoop,
    /// Toggle sync-to-tempo time-stretch of the preview, pushing the new
    /// options to the engine.
    ToggleSync,
    /// Toggle Auto-play-on-select. Pure UI state; not sent to the engine.
    ToggleAutoPlay,
}

impl BrowserMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // Media-browser navigation, filtering, favourite / recent, and
            // audition preview are all transient session UI state (doc #175) —
            // never undoable and never in the project file, same rule as the
            // collapse toggles. Favourites / recent persist to user settings
            // (not the project); the engine's preview transport is outside
            // undo entirely.
            Self::ToggleVisible
            | Self::SelectTab(..)
            | Self::OpenFolder(..)
            | Self::ScanCompleted { .. }
            | Self::SetFilter(..)
            | Self::ToggleFavourite(..)
            | Self::Select(..)
            | Self::Play(..)
            | Self::Stop
            | Self::Scrub(..)
            | Self::ToggleLoop
            | Self::ToggleSync
            | Self::ToggleAutoPlay => UndoAction::Skip,
        }
    }
}

pub fn handle(app: &mut Resonance, message: BrowserMessage) -> Task<Message> {
    match message {
        BrowserMessage::ToggleVisible => {
            app.media.browser.visible = !app.media.browser.visible;
        }

        BrowserMessage::SelectTab(tab) => {
            app.media.browser.tab = tab;
            if tab == crate::state::BrowserTab::Presets {
                crate::update::plugin_preset_ui::media_tab_shown(app);
            }
        }

        BrowserMessage::OpenFolder(path) => return open_folder(app, path),

        BrowserMessage::ScanCompleted { folder, scan } => {
            // Drop a scan whose folder the user has already navigated away
            // from — a newer scan for the current folder is (or will be)
            // in flight and owns the `scanning` flag.
            if app.media.browser.current_folder.as_deref() == Some(folder.as_path()) {
                app.media.browser.scan = scan;
                app.media.browser.scanning = false;
            }
        }

        BrowserMessage::SetFilter(text) => {
            app.media.browser.filter = text;
        }

        BrowserMessage::ToggleFavourite(path) => {
            app.media.pool.toggle_favourite(path);
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
                .media
                .browser
                .audition
                .playing
                .clone()
                .or_else(|| app.media.browser.audition.selected.clone())
            {
                return start_preview(app, path, frame);
            }
        }

        BrowserMessage::ToggleLoop => {
            app.media.browser.audition.loop_enabled = !app.media.browser.audition.loop_enabled;
            send_audition_options(app);
        }

        BrowserMessage::ToggleSync => {
            app.media.browser.audition.sync_to_tempo = !app.media.browser.audition.sync_to_tempo;
            send_audition_options(app);
        }

        BrowserMessage::ToggleAutoPlay => {
            app.media.browser.audition.auto_play = !app.media.browser.audition.auto_play;
        }
    }
    Task::none()
}

/// Navigate the Files tab into `path`: make it the current folder, clear
/// the per-folder filter, record it as most-recently visited (persisted
/// to user settings), and start an off-thread scan.
fn open_folder(app: &mut Resonance, path: PathBuf) -> Task<Message> {
    app.media.browser.current_folder = Some(path.clone());
    app.media.browser.filter.clear();
    // Drop the previous folder's rows so stale content doesn't flash
    // while the new scan runs.
    app.media.browser.scan = FolderScan::default();
    app.media.browser.scanning = true;

    // Visiting a folder pushes it to the recent list (user-level state).
    app.media.pool.push_recent_folder(path.clone());
    app.persist_media_browser_settings();

    scan_task(path)
}

/// Handle a select-to-audition. Highlights `target`; when Auto-play is on
/// a `Some` target immediately previews. A `None` target clears the
/// selection and stops any preview.
fn select(app: &mut Resonance, target: Option<PathBuf>) -> Task<Message> {
    app.media.browser.audition.selected = target.clone();
    match target {
        Some(path) if app.media.browser.audition.auto_play => start_preview(app, path, 0),
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
    app.media.browser.audition.selected = Some(path.clone());
    app.media.browser.audition.playing = Some(path.clone());
    app.media.browser.audition.position_frame = start_frame;
    let _ = app.engine.send(AudioCommand::AuditionFile { path, start_frame });
    Task::none()
}

/// Stop the current preview (if any) and reset the playing row + scrub
/// playhead. A no-op when nothing is playing, mirroring the engine's
/// silent no-op on an idle `StopAudition`.
fn stop_preview(app: &mut Resonance) {
    if app.media.browser.audition.playing.take().is_some() {
        app.media.browser.audition.position_frame = 0;
        let _ = app.engine.send(AudioCommand::StopAudition);
    }
}

/// Push the current loop / sync-to-tempo options to the engine. The engine
/// persists them across `AuditionFile` commands and applies them
/// immediately to any preview already playing.
fn send_audition_options(app: &mut Resonance) {
    let _ = app.engine.send(AudioCommand::SetAuditionOptions {
        loop_enabled: app.media.browser.audition.loop_enabled,
        sync_to_tempo: app.media.browser.audition.sync_to_tempo,
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

    // Decode a compact waveform-thumbnail per audio row so the Files-tab
    // rows (#602) show a real mini silhouette, not a placeholder. This runs
    // in the same off-thread scan as the metadata probe, so it never blocks
    // paint; a file that fails to decode is simply omitted (its row falls
    // back to an idle baseline). The result is cached in `FolderScan` and
    // only recomputed on navigation — the "cache the file-list" performance
    // rule (doc #175).
    let thumbnails = files
        .iter()
        .filter_map(|entry| {
            let thumb =
                resonance_common::waveform_thumbnail(Path::new(&entry.path), THUMBNAIL_BUCKETS)
                    .ok()?;
            let peaks: Vec<(f32, f32)> = thumb
                .min
                .iter()
                .zip(thumb.max.iter())
                .map(|(&lo, &hi)| (lo, hi))
                .collect();
            Some((entry.path.clone(), peaks))
        })
        .collect();

    FolderScan {
        folders,
        files,
        thumbnails,
    }
}

/// Column count for the Files-tab mini waveform thumbnails. Matches the
/// thumbnail widget's ~48 px width so each bucket maps to roughly one
/// pixel column.
const THUMBNAIL_BUCKETS: usize = 48;

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
