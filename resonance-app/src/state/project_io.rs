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
    /// Token of the most recently started disk open. Each open bumps it
    /// and its async load reports back with the value it started with
    /// (`OpenLoadFinished`); a result whose token is no longer current was
    /// overtaken by a later open and is dropped, so two overlapping opens
    /// can't adopt one's content under the other's path (FU-A1a).
    pub open_token: u64,
    /// The `Reconcile` domains the last restore ran, in order, with the
    /// origin it ran them under (ARCH-01 A-13). Cleared at the start of
    /// each restore; read by the order guard test.
    pub reconcile_trace: Vec<(
        crate::update::project_io::reconcile::Origin,
        &'static str,
    )>,
    /// Engine echoes of structural commands a diff restore sent and has
    /// already mirrored (ARCH-01 A-13h). See [`RestoreEchoes`].
    pub restore_echoes: RestoreEchoes,
    pub bouncing: bool,
    /// Progress `[0.0, 1.0]` of the in-flight WAV mixdown (`bouncing`),
    /// from the engine's `BounceProgress` events; drives the blocking
    /// mixdown progress modal (code review FU-F1c).
    pub bounce_fraction: f32,
    /// File name of the in-flight mixdown's target, for the modal title.
    pub bounce_target: String,
    /// The user pressed Cancel on the mixdown modal. The engine answers a
    /// cancel with `BounceError { kind: Cancelled, message: "Bounce
    /// cancelled" }`, which then clears the modal without an error banner.
    pub bounce_cancel_requested: bool,
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

/// Structural echoes a diff restore is still owed by the engine (ARCH-01
/// A-13h).
///
/// The diff restore removes busses and plugin instances and reorders
/// chains itself, mirroring each change the moment it sends the command;
/// the live edit paths instead let the engine's echo do the mirroring.
/// Echoes arrive only after the restore returns, and an undo and a redo
/// can both run before they do (a held Ctrl+Z, a control client's burst).
/// Undo and redo reuse ids by design, so a stale echo can name an entity
/// the *next* restore has put back: `BusRemoved` for a bus redo re-added
/// would delete it, `PluginMoved` would scramble the order redo set, and
/// the `*Added` echo of an instance a later restore removed would push a
/// phantom slot.
///
/// The engine answers every `RemoveBus` / `RemovePlugin*` (an unknown id
/// included) and every `MovePlugin*` of an instance its chain holds, on
/// one FIFO thread, so each expectation is consumed exactly once, in
/// order. A move of an instance whose add failed is answered with an
/// error instead; its expectation is dropped when the `PluginLoadFailed`
/// lands (`forget_plugin_moves`, FU-A13d). The rule the handlers apply:
///
/// * a removal / move echo that matches an expectation is swallowed (the
///   restore already mirrored it);
/// * an add echo for an instance or bus whose removal echo is still owed
///   is ignored — FIFO puts that add *before* the removal the restore
///   already mirrored, so the instance it announces is already gone.
///
/// Counts, not flags: an id can be removed by two restores before either
/// echo lands (remove, re-add, remove).
///
/// Since A-13i the same ledger covers tracks (`TrackRemoved`), audio clips
/// (`ClipDeleted`) and MIDI clips (`MidiClipDeleted`), and not only for a
/// restore: every live edit that mirrors a track or clip deletion at once
/// (STATE-10) owes its echo here too. Before A-13i an undo of such an edit
/// went through `ClearAll`, whose `AllCleared` arrived after the echo; now
/// the diff restore re-adds the entity under the same id at once, and the
/// late echo of the live delete would remove it again.
///
/// A track's own removal-owed flag (`track_removal_owed`) also gates its
/// *scalar* echoes (`TrackFxBypassChanged`, `TrackPlaybackSourceChanged`,
/// FU-A13i): one sent to the old incarnation of an id can otherwise land
/// after a later restore re-added a fresh track under that id and
/// overwrite the value the restore just set, since neither carries an
/// instance-generation marker of its own to tell old from new.
#[derive(Debug, Default)]
pub struct RestoreEchoes {
    removed_busses: std::collections::HashMap<resonance_audio::types::BusId, u32>,
    removed_plugins: std::collections::HashMap<resonance_audio::types::PluginInstanceId, u32>,
    moves: std::collections::HashMap<(resonance_audio::types::PluginInstanceId, usize), u32>,
    added_plugins: std::collections::HashMap<resonance_audio::types::PluginInstanceId, u32>,
    removed_tracks: std::collections::HashMap<resonance_audio::types::TrackId, u32>,
    deleted_clips: std::collections::HashMap<resonance_audio::types::ClipId, u32>,
    deleted_midi_clips: std::collections::HashMap<resonance_audio::types::ClipId, u32>,
}

fn owe<K: std::hash::Hash + Eq>(map: &mut std::collections::HashMap<K, u32>, key: K) {
    *map.entry(key).or_insert(0) += 1;
}

fn settle<K: std::hash::Hash + Eq>(map: &mut std::collections::HashMap<K, u32>, key: K) -> bool {
    let Some(n) = map.get_mut(&key) else {
        return false;
    };
    *n -= 1;
    if *n == 0 {
        map.remove(&key);
    }
    true
}

impl RestoreEchoes {
    /// The restore sent `RemoveBus` and mirrored it.
    pub fn expect_bus_removed(&mut self, bus_id: resonance_audio::types::BusId) {
        owe(&mut self.removed_busses, bus_id);
    }

    /// A `BusRemoved` echo arrived: `true` when a restore owed it (the
    /// caller then ignores it).
    pub fn settle_bus_removed(&mut self, bus_id: resonance_audio::types::BusId) -> bool {
        settle(&mut self.removed_busses, bus_id)
    }

    /// A restore removed this bus and its echo has not arrived yet.
    pub fn bus_removal_owed(&self, bus_id: resonance_audio::types::BusId) -> bool {
        self.removed_busses.contains_key(&bus_id)
    }

    /// The restore sent `RemovePlugin*` and mirrored it.
    pub fn expect_plugin_removed(&mut self, id: resonance_audio::types::PluginInstanceId) {
        owe(&mut self.removed_plugins, id);
    }

    /// A `*PluginRemoved` echo arrived: `true` when a restore owed it.
    pub fn settle_plugin_removed(&mut self, id: resonance_audio::types::PluginInstanceId) -> bool {
        settle(&mut self.removed_plugins, id)
    }

    /// A restore removed this instance and its echo has not arrived yet.
    pub fn plugin_removal_owed(&self, id: resonance_audio::types::PluginInstanceId) -> bool {
        self.removed_plugins.contains_key(&id)
    }

    /// The restore sent `MovePlugin*` to engine index `to_index` and
    /// mirrored it.
    pub fn expect_plugin_moved(&mut self, id: resonance_audio::types::PluginInstanceId, to_index: usize) {
        owe(&mut self.moves, (id, to_index));
    }

    /// A `*PluginMoved` echo arrived: `true` when a restore owed it.
    pub fn settle_plugin_moved(
        &mut self,
        id: resonance_audio::types::PluginInstanceId,
        to_index: usize,
    ) -> bool {
        settle(&mut self.moves, (id, to_index))
    }

    /// A restore added this instance to a chain it kept and counted it as
    /// live when it named the chain's engine indices (FU-A13d). Owed until
    /// the instance's `*PluginAdded` or `PluginLoadFailed` arrives: the
    /// engine answers every add with exactly one of the two.
    pub fn expect_plugin_added(&mut self, id: resonance_audio::types::PluginInstanceId) {
        owe(&mut self.added_plugins, id);
    }

    /// The instance's `*PluginAdded` or `PluginLoadFailed` arrived: `true`
    /// when a restore's add was owed it. A `PluginLoadFailed` that settles
    /// one means the restore's moves counted a plugin the engine does not
    /// have, and the chain has to be put right.
    pub fn settle_plugin_added(&mut self, id: resonance_audio::types::PluginInstanceId) -> bool {
        settle(&mut self.added_plugins, id)
    }

    /// The instance a restore added failed to load: every move the
    /// restore sent for it is answered with an error, not an echo, so none
    /// of them is owed any more.
    pub fn forget_plugin_moves(&mut self, id: resonance_audio::types::PluginInstanceId) {
        self.moves.retain(|(moved, _), _| *moved != id);
    }

    /// `RemoveTrack` was sent and mirrored (by a restore or a live delete).
    /// The engine answers with one `TrackRemoved` for the track and one for
    /// each sub-track it still held under it; each is owed separately.
    pub fn expect_track_removed(&mut self, track_id: resonance_audio::types::TrackId) {
        owe(&mut self.removed_tracks, track_id);
    }

    /// A `TrackRemoved` echo arrived: `true` when it was owed (the caller
    /// then ignores it).
    pub fn settle_track_removed(&mut self, track_id: resonance_audio::types::TrackId) -> bool {
        settle(&mut self.removed_tracks, track_id)
    }

    /// This track's removal echo has not arrived yet: an echo naming it
    /// that arrives now describes the instance already removed (FIFO).
    pub fn track_removal_owed(&self, track_id: resonance_audio::types::TrackId) -> bool {
        self.removed_tracks.contains_key(&track_id)
    }

    /// `DeleteClip` was sent and mirrored.
    pub fn expect_clip_deleted(&mut self, clip_id: resonance_audio::types::ClipId) {
        owe(&mut self.deleted_clips, clip_id);
    }

    /// A `ClipDeleted` echo arrived: `true` when it was owed.
    pub fn settle_clip_deleted(&mut self, clip_id: resonance_audio::types::ClipId) -> bool {
        settle(&mut self.deleted_clips, clip_id)
    }

    /// This audio clip's deletion echo has not arrived yet.
    pub fn clip_deletion_owed(&self, clip_id: resonance_audio::types::ClipId) -> bool {
        self.deleted_clips.contains_key(&clip_id)
    }

    /// `DeleteMidiClip` was sent and mirrored.
    pub fn expect_midi_clip_deleted(&mut self, clip_id: resonance_audio::types::ClipId) {
        owe(&mut self.deleted_midi_clips, clip_id);
    }

    /// A `MidiClipDeleted` echo arrived: `true` when it was owed.
    pub fn settle_midi_clip_deleted(&mut self, clip_id: resonance_audio::types::ClipId) -> bool {
        settle(&mut self.deleted_midi_clips, clip_id)
    }

    /// This MIDI clip's deletion echo has not arrived yet.
    pub fn midi_clip_deletion_owed(&self, clip_id: resonance_audio::types::ClipId) -> bool {
        self.deleted_midi_clips.contains_key(&clip_id)
    }

    /// Nothing is owed.
    pub fn is_empty(&self) -> bool {
        self.removed_busses.is_empty()
            && self.removed_plugins.is_empty()
            && self.moves.is_empty()
            && self.added_plugins.is_empty()
            && self.removed_tracks.is_empty()
            && self.deleted_clips.is_empty()
            && self.deleted_midi_clips.is_empty()
    }
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
