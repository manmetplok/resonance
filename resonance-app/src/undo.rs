//! Session-local undo/redo history.
//!
//! Each undoable action snapshots the declarative project state using the
//! same shape that save/load already understands (`ProjectFile` plus
//! in-memory MIDI notes and cached plugin state blobs). On undo/redo the
//! audio engine is driven back into sync through `replay_loaded_project`,
//! the exact same code path used when opening a project from disk.
//!
//! Phase 1 scope: data types, a bounded history stack, and a snapshot
//! builder on `Resonance`. Message interception, keyboard shortcuts, and
//! the restore path are added in later phases.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use resonance_audio::types::{AudioCommand, ClipId, MidiNote, PluginInstanceId};

use crate::project::{LoadedProject, ProjectFile};
use resonance_audio::types::TrackId;

pub use resonance_audio::DEFAULT_HISTORY_CAPACITY;

/// Runtime-only compose state that isn't captured in `ProjectFile` and
/// therefore can't be rebuilt by `replay_loaded_project` alone. Applied
/// to `Resonance::compose` after the replay completes.
#[derive(Debug, Clone, Default)]
pub struct UndoExtras {
    pub compose_derived_clips: HashMap<(u64, u64, TrackId), ClipId>,
    pub compose_next_derived_clip_id: u64,
    /// Per-clip vocal lyric annotations. Captured so `ToggleSlur` and
    /// any future per-note lyric override edit are reversible. The
    /// `ProjectMidiClip` round-trip writes these on save, but during
    /// a session the undo system snapshots them separately because
    /// the project-file form of a clip isn't rebuilt on each edit.
    pub vocal_clip_lyrics: HashMap<ClipId, Vec<String>>,
}

/// One point in the undo/redo history. Mostly the same shape as
/// `LoadedProject` so snapshots can be fed straight into the existing
/// `replay_loaded_project` path, plus `extras` for runtime-only state.
#[derive(Debug, Clone)]
pub struct UndoSnapshot {
    pub file: ProjectFile,
    pub project_dir: PathBuf,
    pub midi_notes: HashMap<ClipId, Vec<MidiNote>>,
    /// Opaque CLAP state blobs, keyed by plugin instance id. Populated
    /// from `Resonance::plugin_state_cache` at snapshot time; missing
    /// entries cause the restore path to reinstantiate the plugin with
    /// default internal state and rely on the replayed parameter values.
    pub plugin_states: HashMap<PluginInstanceId, Vec<u8>>,
    /// Runtime-only state rebuilt after the replay — currently just the
    /// compose tab's derived-clip cache.
    pub extras: UndoExtras,
}

impl UndoSnapshot {
    /// Split into the `LoadedProject` shape the replay path expects plus
    /// the runtime-only extras, which the caller applies separately after
    /// replay.
    pub fn split(self) -> (LoadedProject, UndoExtras) {
        (
            LoadedProject {
                file: self.file,
                project_dir: self.project_dir,
                midi_notes: self.midi_notes,
                plugin_states: self.plugin_states,
            },
            self.extras,
        )
    }
}

/// Identifies a continuous-edit source so that a stream of messages
/// targeting the same control (a fader drag, a knob twist) collapses into
/// a single undo entry. Any interaction that isn't the same source — a
/// different control, a gesture, a pop, a clear — breaks the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoalesceKey {
    TrackVolume(u64),
    TrackPan(u64),
    BusVolume(u64),
    BusPan(u64),
    MasterVolume,
    PluginParam { instance_id: u64, param_id: u32 },
}

/// Bounded undo / redo stack with a pending-transaction slot for
/// multi-message gestures (clip drag, trim, loop drag, MIDI note drag)
/// and a coalesce slot for knob/fader bursts.
#[derive(Debug, Default)]
pub struct UndoHistory {
    undo: VecDeque<UndoSnapshot>,
    redo: VecDeque<UndoSnapshot>,
    /// Snapshot captured at the start of an in-progress gesture. Committed
    /// to the undo stack on gesture end, discarded on cancel.
    pending: Option<UndoSnapshot>,
    /// Key of the most recently recorded entry, if it was recorded via
    /// `record_coalesced`. Cleared by every other history operation so
    /// any intervening action breaks a coalesce run.
    coalesce_key: Option<CoalesceKey>,
    capacity: usize,
}

impl UndoHistory {
    pub fn new() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            pending: None,
            coalesce_key: None,
            capacity: DEFAULT_HISTORY_CAPACITY,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Record a finished action. Clears the redo stack — any new mutation
    /// invalidates the redo history — and trims to `capacity`.
    pub fn record(&mut self, snapshot: UndoSnapshot) {
        self.undo.push_back(snapshot);
        self.redo.clear();
        self.trim();
        self.coalesce_key = None;
    }

    /// Record an entry that can coalesce with subsequent edits to the
    /// same control. If the last recorded entry was also coalesced under
    /// `key`, this call keeps the existing snapshot (which already
    /// represents the pre-burst state) and only clears the redo stack.
    /// Otherwise a new entry is pushed and the key is remembered.
    pub fn record_coalesced(&mut self, snapshot: UndoSnapshot, key: CoalesceKey) {
        if self.coalesce_key.as_ref() == Some(&key) && !self.undo.is_empty() {
            self.redo.clear();
            return;
        }
        self.undo.push_back(snapshot);
        self.redo.clear();
        self.trim();
        self.coalesce_key = Some(key);
    }

    /// Pop the newest undo entry. The caller is responsible for pushing
    /// the current state onto the redo stack via `push_redo` before
    /// restoring the popped snapshot.
    pub fn pop_undo(&mut self) -> Option<UndoSnapshot> {
        self.coalesce_key = None;
        self.undo.pop_back()
    }

    /// Pop the newest redo entry. The caller is responsible for pushing
    /// the current state onto the undo stack via `push_undo` before
    /// restoring the popped snapshot.
    pub fn pop_redo(&mut self) -> Option<UndoSnapshot> {
        self.coalesce_key = None;
        self.redo.pop_back()
    }

    /// Push a snapshot onto the redo stack without touching the undo stack.
    /// Used when entering an undo: current state goes to redo so it can be
    /// restored by a subsequent redo.
    pub fn push_redo(&mut self, snapshot: UndoSnapshot) {
        self.redo.push_back(snapshot);
        self.coalesce_key = None;
    }

    /// Push a snapshot onto the undo stack without clearing the redo stack.
    /// Used when entering a redo: current state goes to undo so it can be
    /// restored by a subsequent undo.
    pub fn push_undo(&mut self, snapshot: UndoSnapshot) {
        self.undo.push_back(snapshot);
        self.trim();
        self.coalesce_key = None;
    }

    // -- Transaction API for multi-message gestures --------------------

    /// Open a transaction. Used at the start of a drag / trim gesture;
    /// the captured snapshot represents the state before the gesture.
    pub fn begin(&mut self, snapshot: UndoSnapshot) {
        self.pending = Some(snapshot);
        self.coalesce_key = None;
    }

    /// Commit the pending transaction as a single undo entry. Called at
    /// gesture end when the state actually changed.
    pub fn commit(&mut self) {
        if let Some(snap) = self.pending.take() {
            self.record(snap);
        }
    }

    /// Drop the entire history. Called when a new project is loaded —
    /// undo history does not cross the load boundary.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending = None;
        self.coalesce_key = None;
    }

    fn trim(&mut self) {
        while self.undo.len() > self.capacity {
            self.undo.pop_front();
        }
    }
}

impl crate::Resonance {
    /// Build an undo snapshot of the current declarative project state.
    ///
    /// Parameter values come from live GUI state (via
    /// `build_project_file`), so they are always exact. Opaque CLAP state
    /// blobs come from `plugin_state_cache`, which refreshes at natural
    /// resting points — plugin add, editor close, project save — and is
    /// therefore slightly stale between those points.
    pub(crate) fn snapshot_for_undo(&self) -> UndoSnapshot {
        let file = crate::update::build_project_file(self);
        let midi_notes: HashMap<ClipId, Vec<MidiNote>> = self
            .midi_clips
            .iter()
            .map(|mc| (mc.id, mc.notes.clone()))
            .collect();
        let extras = UndoExtras {
            compose_derived_clips: self.compose.derived_clips.clone(),
            compose_next_derived_clip_id: self.compose.next_derived_clip_id,
            vocal_clip_lyrics: self.compose.vocal_audio.clip_lyrics.clone(),
        };
        // Only snapshot blobs for plugins that currently exist — stale
        // entries for removed plugins would bloat the snapshot and are
        // never consumed anyway.
        let mut plugin_states: HashMap<PluginInstanceId, Vec<u8>> = HashMap::new();
        let collect = |slots: &[crate::state::PluginSlotState],
                       out: &mut HashMap<PluginInstanceId, Vec<u8>>,
                       cache: &HashMap<PluginInstanceId, Vec<u8>>| {
            for slot in slots {
                if let Some(blob) = cache.get(&slot.instance_id) {
                    out.insert(slot.instance_id, blob.clone());
                }
            }
        };
        for track in &self.registry.tracks {
            collect(&track.plugins, &mut plugin_states, &self.plugin_state_cache);
        }
        for bus in &self.registry.busses {
            collect(&bus.plugins, &mut plugin_states, &self.plugin_state_cache);
        }
        collect(
            &self.master_plugins,
            &mut plugin_states,
            &self.plugin_state_cache,
        );

        UndoSnapshot {
            file,
            project_dir: self.io.project_path.clone().unwrap_or_default(),
            midi_notes,
            plugin_states,
            extras,
        }
    }

    /// True when the app is in a state where recording a new undo
    /// snapshot would be meaningful. Unsaved projects don't have a
    /// `project_dir` to anchor audio clip paths against, so their
    /// snapshots could never be replayed — there's no point recording
    /// them. Also false during an in-flight restore so intermediate
    /// states mid-replay don't end up in the history.
    pub(crate) fn can_record_undo(&self) -> bool {
        self.io.has_active_project && self.io.project_path.is_some() && !self.io.loading
    }

    /// True when an undo or redo would be safe to start right now. The
    /// recording gate plus: no offline bounce, no in-flight save, no
    /// active recording, and no pending drag/trim transaction (which
    /// would otherwise be silently discarded by the restore).
    pub(crate) fn can_undo_redo_now(&self) -> bool {
        self.can_record_undo()
            && !self.io.bouncing
            && self.io.save_state.is_none()
            && !self.transport.recording
            && !self.undo.has_pending()
    }

    /// Drive the engine and GUI back to `snapshot`. Tries a structure-
    /// preserving diff replay first — when the snapshot has the same set
    /// of tracks, busses, plugins, and clips as the current state, only
    /// the changed scalars (volumes, mutes, BPM, plugin state blobs,
    /// MIDI notes, etc.) are pushed to the engine, keeping every plugin
    /// instance alive. When the structural shape differs, falls back to
    /// the full `ClearAll → AllCleared → replay_loaded_project` pipeline
    /// that `ProjectLoaded(Ok)` uses. Playback is stopped either way
    /// (per v1 policy).
    pub(crate) fn begin_restore_from_snapshot(&mut self, snapshot: UndoSnapshot) {
        // Pause playback and stop recording. Recording should already be
        // blocked by `can_undo_redo_now`, but belt-and-braces.
        self.engine.send(AudioCommand::Stop);
        self.transport.playing = false;
        self.transport.recording = false;

        let (loaded, extras) = snapshot.split();

        // Fast path: structure-identical undo (the common case for
        // fader/knob/transport edits). Drives the engine surgically
        // without tearing down plugin instances.
        if crate::update::try_diff_replay(self, &loaded, &extras) {
            return;
        }

        // Slow path: structural change. Stash both halves so the
        // `AllCleared` handler can run the full replay. The handler
        // re-establishes `project_path` from the snapshot's project_dir
        // because `replay_loaded_project` clears it on entry.
        self.io.loading = true;
        self.io.pending_load = Some(Box::new(loaded));
        self.io.pending_undo_extras = Some(extras);

        self.engine.send(AudioCommand::ClearAll);
    }

    /// Apply the runtime-only extras captured in the snapshot. Called
    /// from the `AllCleared` engine-event handler immediately after
    /// `replay_loaded_project` runs, only when the pending load came
    /// from an undo/redo (distinguished by `pending_undo_extras.is_some()`).
    pub(crate) fn finalize_undo_restore(&mut self, extras: UndoExtras) {
        self.compose.derived_clips = extras.compose_derived_clips;
        self.compose.next_derived_clip_id = extras.compose_next_derived_clip_id;
        self.compose.vocal_audio.clip_lyrics = extras.vocal_clip_lyrics;
    }

    /// Attempt to undo. No-ops (returning false) if the history is empty
    /// or an in-flight operation blocks undo/redo. On success the current
    /// state is pushed onto the redo stack before the snapshot is
    /// restored.
    pub(crate) fn try_undo(&mut self) -> bool {
        if !self.can_undo_redo_now() || !self.undo.can_undo() {
            return false;
        }
        let Some(snapshot) = self.undo.pop_undo() else {
            return false;
        };
        let current = self.snapshot_for_undo();
        self.undo.push_redo(current);
        self.begin_restore_from_snapshot(snapshot);
        true
    }

    /// Symmetric counterpart to `try_undo`.
    pub(crate) fn try_redo(&mut self) -> bool {
        if !self.can_undo_redo_now() || !self.undo.can_redo() {
            return false;
        }
        let Some(snapshot) = self.undo.pop_redo() else {
            return false;
        };
        let current = self.snapshot_for_undo();
        self.undo.push_undo(current);
        self.begin_restore_from_snapshot(snapshot);
        true
    }
}

// ---------------------------------------------------------------------
// Message classifier
// ---------------------------------------------------------------------

/// What the undo system should do with an incoming message. Computed from
/// the message variant alone — no access to state — so it runs at the
/// top of `update()` with no borrow conflicts.
#[derive(Debug, Clone)]
pub enum UndoAction {
    /// Don't touch the history. UI-only, engine echoes, mid-gesture
    /// updates, transient runtime messages.
    Skip,
    /// Record a new atomic undo entry, capturing the pre-dispatch state.
    Record,
    /// Record an entry that coalesces with subsequent edits using the
    /// same `CoalesceKey` — for fader/knob bursts.
    RecordCoalesced(CoalesceKey),
    /// Open a transaction: snapshot the pre-gesture state. Committed by
    /// the matching gesture-end message.
    Begin,
    /// Commit the pending transaction opened by an earlier `Begin`.
    Commit,
}

/// Decide how an incoming message should interact with the undo history.
pub fn classify(message: &crate::message::Message) -> UndoAction {
    use crate::compose::ComposeMessage;
    use crate::message::*;

    match message {
        // Meta-messages never reach the classifier — update() handles
        // them before calling classify — but be defensive.
        Message::Undo | Message::Redo => UndoAction::Skip,

        // Window close request: pure UI flow, no project mutation.
        Message::WindowCloseRequested(_) => UndoAction::Skip,

        // Timer tick, pure UI, engine runtime, project I/O.
        Message::Tick => UndoAction::Skip,
        Message::Viewport(_) => UndoAction::Skip,
        Message::Ui(_) => UndoAction::Skip,
        Message::ProjectIo(_) => UndoAction::Skip,
        Message::GlobalTrack(GlobalTrackMessage::SelectEvent(_)) => UndoAction::Skip,
        Message::GlobalTrack(GlobalTrackMessage::StartTempoDrag(_)) => UndoAction::Begin,
        Message::GlobalTrack(GlobalTrackMessage::EndTempoDrag) => UndoAction::Commit,
        Message::GlobalTrack(GlobalTrackMessage::UpdateTempoEvent { .. }) => UndoAction::Skip,
        Message::GlobalTrack(_) => UndoAction::Record,

        Message::Transport(t) => match t {
            TransportMessage::StartLoopDrag(_) => UndoAction::Begin,
            TransportMessage::EndLoopDrag => UndoAction::Commit,
            TransportMessage::UpdateLoopDrag(_) => UndoAction::Skip,
            TransportMessage::Play
            | TransportMessage::Record
            | TransportMessage::Pause
            | TransportMessage::Stop
            | TransportMessage::SkipBack
            | TransportMessage::SkipForward
            | TransportMessage::SeekToSample(_)
            | TransportMessage::SetBpmText(_) => UndoAction::Skip,
            TransportMessage::CommitBpm
            | TransportMessage::ToggleMetronome
            | TransportMessage::CycleTimeSignature
            | TransportMessage::ToggleLoop => UndoAction::Record,
        },

        Message::Track(t) => match t {
            TrackMessage::SetTrackVolume(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::TrackVolume(*id))
            }
            TrackMessage::SetTrackPan(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::TrackPan(*id))
            }
            TrackMessage::SetMasterVolume(_) => {
                UndoAction::RecordCoalesced(CoalesceKey::MasterVolume)
            }
            TrackMessage::ToggleSubTracksVisible(_) => UndoAction::Skip,
            // Dismissing the delete-confirmation dialog is a transient
            // UI gesture — nothing to undo.
            TrackMessage::CancelRemoveTrack => UndoAction::Skip,
            // Preset operations that don't mutate project state.
            TrackMessage::DeleteUserPreset(_) => UndoAction::Skip,
            _ => UndoAction::Record,
        },

        Message::Bus(b) => match b {
            BusMessage::SetBusVolume(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::BusVolume(*id))
            }
            BusMessage::SetBusPan(id, _) => UndoAction::RecordCoalesced(CoalesceKey::BusPan(*id)),
            _ => UndoAction::Record,
        },

        Message::Master(_) => UndoAction::Record,

        Message::Plugin(p) => match p {
            PluginMessage::AddPluginToTrack(_, _) | PluginMessage::RemovePluginFromTrack(_, _) => {
                UndoAction::Record
            }
            PluginMessage::SetPluginParam(instance_id, param_id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::PluginParam {
                    instance_id: *instance_id,
                    param_id: *param_id,
                })
            }
            PluginMessage::TogglePluginPanel(_)
            | PluginMessage::OpenPluginEditor(_)
            | PluginMessage::ClosePluginEditor(_) => UndoAction::Skip,
        },

        Message::Clip(c) => match c {
            ClipMessage::StartClipDrag { .. } | ClipMessage::StartClipTrim { .. } => {
                UndoAction::Begin
            }
            ClipMessage::EndClipDrag | ClipMessage::EndClipTrim => UndoAction::Commit,
            ClipMessage::UpdateClipDrag(_, _) | ClipMessage::UpdateClipTrim(_) => UndoAction::Skip,
            ClipMessage::DeleteClip(_) => UndoAction::Record,
        },

        Message::MidiClip(c) => match c {
            MidiClipMessage::StartMidiClipDrag { .. }
            | MidiClipMessage::StartMidiClipTrim { .. } => UndoAction::Begin,
            MidiClipMessage::EndMidiClipDrag | MidiClipMessage::EndMidiClipTrim => {
                UndoAction::Commit
            }
            MidiClipMessage::UpdateMidiClipDrag(_, _) | MidiClipMessage::UpdateMidiClipTrim(_) => {
                UndoAction::Skip
            }
            MidiClipMessage::DeleteMidiClip(_) => UndoAction::Record,
        },

        Message::MidiEditor(e) => match e {
            MidiEditorMessage::AddNote { .. }
            | MidiEditorMessage::RemoveNote { .. }
            | MidiEditorMessage::MoveNote { .. }
            | MidiEditorMessage::ResizeNote { .. }
            | MidiEditorMessage::ToggleSlur { .. } => UndoAction::Record,
            MidiEditorMessage::OpenMidiEditor(_)
            | MidiEditorMessage::OpenSelectedMidiClip
            | MidiEditorMessage::CloseMidiEditor
            | MidiEditorMessage::SelectNote { .. }
            | MidiEditorMessage::PreviewNote(_, _)
            | MidiEditorMessage::StopPreview(_, _)
            | MidiEditorMessage::ScrollY(_) => UndoAction::Skip,
        },

        Message::Compose(c) => match c {
            // Form input, selections, panel open/close: UI only.
            ComposeMessage::OpenCreateSectionDialog
            | ComposeMessage::CancelCreateSectionDialog
            | ComposeMessage::SetNewSectionName(_)
            | ComposeMessage::SetNewSectionLength(_)
            | ComposeMessage::OpenEditSectionDialog { .. }
            | ComposeMessage::CancelEditSectionDialog
            | ComposeMessage::SetEditSectionName(_)
            | ComposeMessage::SetEditSectionLength(_)
            | ComposeMessage::SelectSectionPlacement { .. }
            | ComposeMessage::SelectChord { .. }
            | ComposeMessage::ClearChordSelection
            | ComposeMessage::SelectLane(_)
            | ComposeMessage::ExpandTrack { .. }
            | ComposeMessage::CollapseTrack
            | ComposeMessage::ExpandedScrollX(_)
            | ComposeMessage::ExpandedScrollY(_)
            | ComposeMessage::ExpandedZoomY(_) => UndoAction::Skip,

            // Everything else in Compose mutates project state.
            _ => UndoAction::Record,
        },
    }
}

// Inline tests: `resonance-app` is a binary crate with no `lib.rs`. These
// tests poke private fields (`UndoHistory::capacity`, `undo`, `redo`) to
// verify capacity trimming and coalesce-key behaviour without exposing
// internals through the public API. See ARCHITECTURE.md → Test Layout →
// Binary-crate exception.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::PROJECT_FORMAT_VERSION;

    /// Produce a snapshot that carries `id` in its `file.bpm` field so
    /// tests can distinguish snapshots on the history stack. `bpm` is
    /// abused purely as a numeric discriminator here; the rest of the
    /// snapshot is a valid default.
    fn dummy_snapshot(id: f32) -> UndoSnapshot {
        UndoSnapshot {
            file: ProjectFile {
                version: PROJECT_FORMAT_VERSION,
                sample_rate: 44100,
                bpm: id,
                time_sig_num: 4,
                time_sig_den: 4,
                metronome_enabled: false,
                master_volume: 0.0,
                master_plugins: Vec::new(),
                master_fx_bypassed: false,
                loop_enabled: false,
                loop_in: 0,
                loop_out: 0,
                tracks: Vec::new(),
                clips: Vec::new(),
                midi_clips: Vec::new(),
                busses: Vec::new(),
                section_definitions: Vec::new(),
                section_placements: Vec::new(),
                tempo_events: Vec::new(),
                signature_events: Vec::new(),
                midi_clock_send_enabled: false,
                midi_clock_send_device: None,
                midi_clock_recv_enabled: false,
                midi_clock_recv_device: None,
                drum_groups: Vec::new(),
                drum_patterns: Vec::new(),
            },
            project_dir: PathBuf::new(),
            midi_notes: HashMap::new(),
            plugin_states: HashMap::new(),
            extras: UndoExtras::default(),
        }
    }

    #[test]
    fn record_clears_redo() {
        let mut h = UndoHistory::new();
        h.record(dummy_snapshot(1.0));
        // Simulate an undo: redo now has one entry.
        let popped = h.pop_undo().unwrap();
        h.push_redo(popped);
        assert!(h.can_redo());
        // Recording a new action must wipe redo.
        h.record(dummy_snapshot(2.0));
        assert!(!h.can_redo());
    }

    #[test]
    fn capacity_trims_oldest() {
        let mut h = UndoHistory::new();
        h.capacity = 3;
        h.record(dummy_snapshot(1.0));
        h.record(dummy_snapshot(2.0));
        h.record(dummy_snapshot(3.0));
        h.record(dummy_snapshot(4.0));
        assert_eq!(h.undo.len(), 3);
        // The oldest (1.0) should have been trimmed; top of stack is 4.0.
        assert_eq!(h.pop_undo().unwrap().file.bpm, 4.0);
        assert_eq!(h.pop_undo().unwrap().file.bpm, 3.0);
        assert_eq!(h.pop_undo().unwrap().file.bpm, 2.0);
        assert!(h.pop_undo().is_none());
    }

    #[test]
    fn commit_records_pending_transaction() {
        let mut h = UndoHistory::new();
        h.begin(dummy_snapshot(1.0));
        assert!(h.has_pending());
        h.commit();
        assert!(!h.has_pending());
        assert!(h.can_undo());
        assert_eq!(h.undo[0].file.bpm, 1.0);
    }

    #[test]
    fn coalesces_same_key_and_breaks_on_intervening_action() {
        let mut h = UndoHistory::new();
        let key = CoalesceKey::TrackVolume(7);

        // First entry under `key` pushes normally.
        h.record_coalesced(dummy_snapshot(1.0), key.clone());
        assert_eq!(h.undo.len(), 1);
        // Subsequent entries under the same key do not push.
        h.record_coalesced(dummy_snapshot(2.0), key.clone());
        h.record_coalesced(dummy_snapshot(3.0), key.clone());
        assert_eq!(h.undo.len(), 1);
        // The retained entry is the original (pre-burst) snapshot.
        assert_eq!(h.undo[0].file.bpm, 1.0);

        // A different coalesce key breaks the run and pushes a new entry.
        h.record_coalesced(dummy_snapshot(10.0), CoalesceKey::TrackPan(7));
        assert_eq!(h.undo.len(), 2);

        // An atomic record also breaks any subsequent coalesce run.
        h.record(dummy_snapshot(20.0));
        h.record_coalesced(dummy_snapshot(4.0), key);
        assert_eq!(h.undo.len(), 4);
    }

    #[test]
    fn coalesce_run_is_broken_by_pop() {
        let mut h = UndoHistory::new();
        let key = CoalesceKey::MasterVolume;
        h.record_coalesced(dummy_snapshot(1.0), key.clone());
        h.pop_undo();
        // After popping, the next coalesced record must push fresh.
        h.record_coalesced(dummy_snapshot(2.0), key);
        assert_eq!(h.undo.len(), 1);
        assert_eq!(h.undo[0].file.bpm, 2.0);
    }

    #[test]
    fn clear_empties_everything() {
        let mut h = UndoHistory::new();
        h.record(dummy_snapshot(1.0));
        h.begin(dummy_snapshot(2.0));
        let snap = h.pop_undo().unwrap();
        h.push_redo(snap);
        h.clear();
        assert!(!h.can_undo());
        assert!(!h.can_redo());
        assert!(!h.has_pending());
    }
}
