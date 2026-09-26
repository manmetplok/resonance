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
    /// Target of a disk open whose async load is still in flight. Only
    /// adopted as `project_path` (and sent as the engine's project dir)
    /// once the load succeeds, so a failed open leaves the current
    /// project's path alone (code review STATE-01 / UPD-01).
    pub pending_open_path: Option<std::path::PathBuf>,
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
    /// The control revision at which the in-flight manual save captured
    /// the project (`try_finish_save`). Its completion clears `dirty` only
    /// when the revision is unchanged: an edit made while the files were
    /// being written is not in them (code review STATE-09).
    pub save_capture_revision: Option<u64>,
    /// A manual save was requested while another save's engine round-trip
    /// was collecting; it starts once that collector completes (code
    /// review STATE-11).
    pub manual_save_queued: bool,
    /// When the project last went from clean to dirty, as seen by the
    /// autosave trigger: the first interval runs from here (UPD-07).
    pub autosave_armed_at: Option<std::time::SystemTime>,
    /// The revision the last autosave snapshot started at; an unchanged
    /// project is not snapshotted again.
    pub autosave_revision: Option<u64>,
    /// The directory this session's crash-detection marker is in: the
    /// open project's dir, or the untitled autosave scratch dir
    /// (`update::project_io::recovery`, code review FU-M12a).
    pub session_marker_dir: Option<std::path::PathBuf>,
    /// The autosave-recovery prompt, while it is open.
    pub recovery_prompt: Option<RecoveryPrompt>,
    /// What the in-flight disk load means for recovery; consumed when the
    /// loaded project has replayed.
    pub load_recovery: Option<LoadRecovery>,
    /// A crashed untitled session's scratch dir this session recovered
    /// from. Its files stay until the next successful save or a clean
    /// quit.
    pub recovered_scratch_dir: Option<std::path::PathBuf>,
}

/// The autosave-recovery prompt's subject (code review FU-M12a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryPrompt {
    pub offer: crate::project::session::RecoveryOffer,
    /// A never-saved session found in the scratch root at startup, rather
    /// than a project being opened: there is no "last saved" version.
    pub untitled: bool,
}

/// Recovery facts about a disk load in flight.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadRecovery {
    /// The load is of the autosave: the project lands dirty.
    pub recovered: bool,
    /// A recoverable autosave exists (reported to a control client).
    pub autosave_available: bool,
    /// The crashed untitled session's scratch dir being recovered.
    pub scratch_dir: Option<std::path::PathBuf>,
}
