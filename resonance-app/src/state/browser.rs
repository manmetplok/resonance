//! Transient media-browser interaction state (doc #175, todo #599).
//!
//! Everything here is *session UI state* — like the collapse state of an
//! inspector group, it is deliberately **not undoable and not persisted
//! in the project file**. Navigating folders, filtering, switching the
//! Files/Pool tab, and auditioning a preview all mutate only this struct
//! and (for auditioning) drive the engine's preview transport; none of it
//! touches the `ProjectFile` snapshot the undo history takes.
//!
//! The two *durable* things the browser reads — the media pool's assets
//! and the favourite / recent folder lists — live on [`crate::state::pool`]
//! (assets ride the project file; favourites / recent ride user settings).
//! Toggling a favourite or visiting a folder updates those lists and
//! writes them back to `settings.json`, which is user-level state, *not*
//! project persistence — the same rule the pool module already documents.
//!
//! The folder scan itself (subfolders + probed audio rows) is run
//! off-thread — see `update::browser` — so a slow directory never blocks
//! the UI thread. Its result is cached here in [`BrowserState::scan`] and
//! only replaced when the returned folder still matches the one the user
//! is looking at (a stale scan from a folder they've since left is
//! dropped).

use std::path::{Path, PathBuf};

use resonance_common::audio_probe::AudioFileEntry;

/// The two media-browser tabs. *Files* browses the filesystem (breadcrumb
/// + folder / audio rows); *Pool* lists the project's already-imported
/// assets. Only the Files tab uses the folder-navigation and scan state;
/// the Pool tab is driven from [`crate::state::pool::MediaPool`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowserTab {
    /// Filesystem browse: breadcrumb, favourites / recent shelf, folder
    /// and audio rows for the current folder.
    #[default]
    Files,
    /// The project's media pool: imported assets with usage counts.
    Pool,
}

/// A completed folder scan: the child folders and the probed audio files
/// of one directory. Produced off-thread by
/// [`crate::update::browser::scan_folder`] and cached in
/// [`BrowserState::scan`]. Both lists are sorted by path so the view can
/// render them in a stable order without re-sorting each frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FolderScan {
    /// Immediate child directories of the scanned folder, sorted by path.
    pub folders: Vec<PathBuf>,
    /// Audio files in the scanned folder with probed metadata, sorted by
    /// path. See [`resonance_common::scan_audio_folder`].
    pub files: Vec<AudioFileEntry>,
}

/// Audition-preview transport state for the browser's bottom bar.
///
/// Auditioning plays an arbitrary audio file (a filesystem row or a pool
/// asset) through the engine's dedicated preview path — independent of the
/// arrangement, transport, and undo (see [`AudioCommand::AuditionFile`]).
/// This mirror lets the view highlight the playing row, draw the scrub
/// playhead, and reflect the loop / sync / auto-play toggles. The
/// `position_frame` scrub playhead is refreshed from `AuditionPosition`
/// engine events (mirrored in todo #597); everything else is set by the
/// [`crate::message::BrowserMessage`] handlers here.
///
/// [`AudioCommand::AuditionFile`]: resonance_audio::types::AudioCommand::AuditionFile
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuditionState {
    /// The row the user has selected to audition, if any. Selecting a row
    /// highlights it and — when [`Self::auto_play`] is on — immediately
    /// starts previewing it. Cleared when the user selects nothing.
    pub selected: Option<PathBuf>,
    /// The row currently sounding through the engine preview, if any.
    /// `Some` between a `Play` and the matching `Stop` / natural end
    /// (the `AuditionStopped` event, mirrored in #597). Drives the WARM
    /// "playing" row highlight.
    pub playing: Option<PathBuf>,
    /// Preview playhead in source frames, updated from throttled
    /// `AuditionPosition` events while a preview plays. Reset to 0 when a
    /// new preview starts. This is a live readout — keep it out of any
    /// `lazy` region whose fingerprint omits it (doc #175).
    pub position_frame: u64,
    /// Loop the preview at its end instead of stopping. Sent to the engine
    /// via [`SetAuditionOptions`] and persisted across previews there.
    ///
    /// [`SetAuditionOptions`]: resonance_audio::types::AudioCommand::SetAuditionOptions
    pub loop_enabled: bool,
    /// Time-stretch a looped preview so its length snaps to the project
    /// tempo. Sent to the engine alongside [`Self::loop_enabled`].
    pub sync_to_tempo: bool,
    /// When on, selecting a row starts previewing it straight away. Pure
    /// UI state — never sent to the engine.
    pub auto_play: bool,
}

impl AuditionState {
    /// True when `path` is the row currently sounding.
    pub fn is_playing(&self, path: &Path) -> bool {
        self.playing.as_deref() == Some(path)
    }

    /// True when `path` is the selected-to-audition row.
    pub fn is_selected(&self, path: &Path) -> bool {
        self.selected.as_deref() == Some(path)
    }
}

/// Transient state for the docked media browser: which tab is showing,
/// the current filesystem folder + its cached scan, the per-folder text
/// filter, and the audition transport. Held in `Resonance` state; reset
/// implicitly by dropping the whole struct — nothing here is serialized.
#[derive(Debug, Clone, Default)]
pub struct BrowserState {
    /// Files vs Pool tab.
    pub tab: BrowserTab,
    /// The folder the Files tab is currently showing. `None` before the
    /// user has navigated anywhere (the shelf of favourites / recent
    /// folders is the entry point).
    pub current_folder: Option<PathBuf>,
    /// Cached scan (subfolders + audio rows) of [`Self::current_folder`].
    /// Replaced only by a `ScanCompleted` whose folder still matches the
    /// current one; empty while no folder is open.
    pub scan: FolderScan,
    /// True while an off-thread scan of the current folder is in flight,
    /// so the view can show a spinner / "scanning…" affordance.
    pub scanning: bool,
    /// Per-folder case-insensitive substring filter applied to the file
    /// rows. Reset to empty on every folder navigation.
    pub filter: String,
    /// Audition-preview transport state.
    pub audition: AuditionState,
}

impl BrowserState {
    /// The breadcrumb chain for the current folder: every ancestor from
    /// the filesystem root down to and including the current folder, in
    /// that order. Empty when no folder is open. The view renders one
    /// clickable crumb per entry (each is a valid `OpenFolder` target).
    pub fn breadcrumb(&self) -> Vec<PathBuf> {
        let Some(folder) = self.current_folder.as_ref() else {
            return Vec::new();
        };
        let mut crumbs: Vec<PathBuf> = folder.ancestors().map(Path::to_path_buf).collect();
        // `ancestors()` yields current-folder-first; reverse to root-first.
        crumbs.reverse();
        crumbs
    }

    /// Iterator over the current folder's audio rows that pass the active
    /// text filter (case-insensitive substring match on the file name).
    /// An empty filter passes everything.
    pub fn filtered_files(&self) -> impl Iterator<Item = &AudioFileEntry> {
        let needle = self.filter.trim().to_lowercase();
        self.scan.files.iter().filter(move |entry| {
            if needle.is_empty() {
                return true;
            }
            file_name_lower(&entry.path).contains(&needle)
        })
    }
}

/// Lower-cased final path component of `path` (the file / folder name),
/// for case-insensitive filter matching. Falls back to the whole string
/// when there is no final component.
fn file_name_lower(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_lowercase()
}
