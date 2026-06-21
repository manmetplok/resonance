//! Project save/load and bounce-progress state. Held on `Resonance` as a
//! single sub-struct so the open/save/load/bounce code path doesn't pull
//! in the rest of the GUI state.

/// Whether an in-flight bounce is rendering offline (CLAP synth) or
/// recording in real time from an audio input. Drives the progress
/// modal's wording and gates which features the cancel button enables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BounceMode {
    Offline,
    Realtime,
}

/// Active state for the bounce-in-place progress modal.
#[derive(Debug, Clone)]
pub struct BounceProgressState {
    pub mode: BounceMode,
    /// Display name of the source track (used in the modal title).
    pub source_name: String,
    /// `[0.0, 1.0]` from the engine's `BounceProgress` events.
    pub fraction: f32,
}

/// Transient state for the dialog. Lives on `Resonance::bounce_dialog`
/// while the overlay is open; the realtime bounce kicks off when the
/// user confirms with `selected_device` set.
#[derive(Debug, Clone)]
pub struct BounceDialogState {
    pub source_track_id: resonance_audio::types::TrackId,
    /// Selected input device name. `None` until the user picks one.
    pub selected_device: Option<String>,
    /// Selected starting input channel (0-indexed). Defaults to 0. In
    /// stereo mode the right channel is `selected_port + 1`.
    pub selected_port: u16,
    /// Capture as mono (single channel duplicated to L/R) vs stereo
    /// (a pair of consecutive channels). Defaults to stereo because
    /// almost every external instrument returns a stereo pair.
    pub mono: bool,
}

/// Project save/load and offline-bounce progress state.
#[derive(Default)]
pub struct ProjectIoState {
    pub project_path: Option<std::path::PathBuf>,
    pub save_state: Option<crate::project::SaveCollector>,
    pub loading: bool,
    pub pending_load: Option<Box<crate::project::LoadedProject>>,
    /// Runtime-only state to re-apply after an undo/redo restore, once
    /// `replay_loaded_project` has rebuilt the declarative project.
    /// `None` for a normal project load, `Some` for undo/redo.
    pub pending_undo_extras: Option<crate::undo::UndoExtras>,
    pub bouncing: bool,
    /// When false, the startup modal is shown and interactive
    /// messages are dropped. Flipped true on successful load or
    /// on the first successful save of a new project.
    pub has_active_project: bool,
    /// Recent-projects list, loaded from disk on startup and
    /// refreshed whenever an entry is added.
    pub recent_projects: Vec<crate::recent::RecentEntry>,
    /// Wall-clock time of the last successful clean (manual) save,
    /// driving the "last saved" chrome indicator. `None` until the
    /// first save of this session.
    pub last_saved_at: Option<std::time::SystemTime>,
    /// Wall-clock time of the last successful autosave snapshot. Tracked
    /// separately from [`Self::last_saved_at`] because an autosave does
    /// not clear `dirty` and the UI distinguishes the two.
    pub last_autosave_at: Option<std::time::SystemTime>,
    /// True while a manual save or autosave is writing to disk. Distinct
    /// from the `dirty` flag: a project can be dirty with no save in
    /// flight, and a save can be in flight on a project that is no longer
    /// dirty. Drives the in-progress spinner/affordance in the chrome.
    pub saving: bool,
}
