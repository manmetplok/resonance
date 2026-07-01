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

use resonance_audio::types::{AudioCommand, ClipId, FadeCurve, MidiNote, PluginInstanceId};
use resonance_common::ExternalInstrument;

use crate::project::LoadedProject;
use resonance_audio::types::TrackId;

pub use resonance_audio::DEFAULT_HISTORY_CAPACITY;

/// Per-clip fade + gain values captured for undo. Mirrors the editable
/// fields on [`crate::state::ClipState`] (and on the engine's `AudioClip`).
/// These don't ride the `ProjectFile` snapshot yet — clip fade/gain
/// persistence is a separate todo (doc #156 A6 / #321) — so, exactly like
/// `reference` / `chord_track`, the undoable set is captured here and
/// re-applied (mirror + engine re-sync) by the restore paths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipFadeGain {
    pub fade_in_frames: u64,
    pub fade_in_curve: FadeCurve,
    pub fade_out_frames: u64,
    pub fade_out_curve: FadeCurve,
    pub gain_db: f32,
}

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
    /// Reference-track (A/B) content. References aren't part of the
    /// `ProjectFile` yet, so `replay_loaded_project` can't rebuild them;
    /// the undoable subset is snapshotted here and reapplied after the
    /// replay (both the fast diff path and the full-clear path).
    pub reference: crate::reference::ReferenceUndo,
    /// The global chord track (epic #33). Captured here rather than in
    /// `ProjectFile` because chord-track persistence is a later todo;
    /// until then the track is declarative app state that the replay
    /// path can't rebuild, so undo snapshots it directly.
    pub chord_track: crate::chord_track::ChordTrack,
    /// Full drum arrangement per section definition. The project-file
    /// form still flattens each arrangement to its primary pattern id
    /// (multi-entry persistence is a separate todo), so the snapshot
    /// captures the complete `Vec<PatternEntry>` here to make
    /// arrangement edits — reorder, fills, length modes, multi-entry —
    /// fully reversible without waiting on disk persistence.
    pub compose_arrangements: HashMap<u64, Vec<crate::compose::PatternEntry>>,
    /// Per-track freeze status at snapshot time. The rendered cache is not
    /// part of undo history, so on restore
    /// [`crate::Resonance::apply_freeze_restore`] detaches + deletes the
    /// cache of any track that is no longer frozen and downgrades a
    /// re-frozen track whose cache file is gone to stale.
    pub track_freeze: HashMap<TrackId, crate::state::FreezeStatus>,
    /// Per-clip fade + gain at snapshot time (doc #156 A2/#317). Captured
    /// here because clip fade/gain isn't part of `ProjectFile` yet (the
    /// persistence slice is #321); the restore paths re-apply each entry to
    /// the `ClipState` mirror and re-sync the engine via `SetClipFade` /
    /// `SetClipGain`, making fade/gain edits fully reversible without
    /// waiting on disk persistence. Only clips present at restore time are
    /// touched (a clip removed by the same undo is handled by the
    /// structural replay path).
    pub clip_fade_gain: HashMap<ClipId, ClipFadeGain>,
    /// External-instrument config per track (bank/program/latency + the
    /// external-mode marker). Captured here because the `ProjectFile` shape
    /// doesn't carry it yet (project persistence lands in a later todo), so
    /// the undo system snapshots it separately — exactly like
    /// `vocal_clip_lyrics`. The runtime device-offline flags are *not*
    /// captured: they reflect live hardware, not project state.
    pub external_instruments: HashMap<TrackId, ExternalInstrument>,
}

/// Re-apply the snapshotted full arrangements onto the compose state after
/// a project replay. The replay path rebuilds each section's arrangement
/// from the persisted (flattened) primary pattern id; this overwrites it
/// with the captured `Vec<PatternEntry>` so multi-entry arrangements,
/// fills, and `Bars` length modes survive an undo/redo. Sections present
/// in the live state but missing from the snapshot are left untouched.
pub(crate) fn restore_arrangements(
    compose: &mut crate::compose::ComposeState,
    arrangements: &HashMap<u64, Vec<crate::compose::PatternEntry>>,
) {
    for (id, arrangement) in arrangements {
        if let Some(def) = compose.find_definition_mut(*id) {
            def.arrangement = arrangement.clone();
        }
    }
}

/// One point in the undo/redo history. Wraps the `LoadedProject` shape
/// so snapshots can be fed straight into the existing
/// `replay_loaded_project` path, plus `extras` for runtime-only state.
#[derive(Debug, Clone)]
pub struct UndoSnapshot {
    /// Declarative project state in the exact shape the replay path
    /// expects. `plugin_states` is populated from
    /// `Resonance::plugin_state_cache` at snapshot time; missing entries
    /// cause the restore path to reinstantiate the plugin with default
    /// internal state and rely on the replayed parameter values.
    pub project: LoadedProject,
    /// Runtime-only state rebuilt after the replay — currently just the
    /// compose tab's derived-clip cache.
    pub extras: UndoExtras,
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
    /// The reference-track manual trim fader.
    ReferenceTrim,
    /// An aux-send level slider drag, keyed by the send's id.
    SendLevel(u64),
    /// Dragging a marker's start pole along the ruler, keyed by marker id.
    MarkerMove(u64),
    /// Dragging a region marker's end edge, keyed by marker id.
    MarkerResize(u64),
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

    // ---- Test-only accessors for integration tests -------------------
    //
    // `tests/undo_history.rs` verifies capacity trimming and coalesce-key
    // behaviour, which requires poking the private `capacity` / `undo`
    // fields. Same `#[doc(hidden)]` convention as the `test_*` accessors
    // on `Resonance` in `lib.rs`: not part of the user-facing surface,
    // crate-internal code keeps using the private fields directly.

    /// Test-only: override the history capacity so trimming is testable
    /// without recording `DEFAULT_HISTORY_CAPACITY` snapshots.
    #[doc(hidden)]
    pub fn test_set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
    }

    /// Test-only: read the undo stack (oldest first) so tests can assert
    /// entry counts and inspect retained snapshots.
    #[doc(hidden)]
    pub fn test_undo_entries(&self) -> &VecDeque<UndoSnapshot> {
        &self.undo
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
            reference: self.reference.undo_snapshot(),
            chord_track: self.chord_track.clone(),
            compose_arrangements: self
                .compose
                .definitions
                .iter()
                .map(|d| (d.id, d.arrangement.clone()))
                .collect(),
            track_freeze: self.freeze.statuses.clone(),
            clip_fade_gain: self
                .clips
                .iter()
                .map(|c| {
                    (
                        c.id,
                        ClipFadeGain {
                            fade_in_frames: c.fade_in_frames,
                            fade_in_curve: c.fade_in_curve,
                            fade_out_frames: c.fade_out_frames,
                            fade_out_curve: c.fade_out_curve,
                            gain_db: c.gain_db,
                        },
                    )
                })
                .collect(),
            external_instruments: self
                .external_instruments
                .iter()
                .map(|(id, st)| (*id, st.config()))
                .collect(),
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
            project: LoadedProject {
                file,
                project_dir: self.io.project_path.clone().unwrap_or_default(),
                midi_notes,
                plugin_states,
            },
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
            && !self.freeze.any_in_flight()
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
        let _ = self.engine.send(AudioCommand::Stop);
        self.transport.playing = false;
        self.transport.recording = false;

        let UndoSnapshot {
            project: loaded,
            extras,
        } = snapshot;

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

        let _ = self.engine.send(AudioCommand::ClearAll);
    }

    /// Apply the runtime-only extras captured in the snapshot. Called
    /// from the `AllCleared` engine-event handler immediately after
    /// `replay_loaded_project` runs, only when the pending load came
    /// from an undo/redo (distinguished by `pending_undo_extras.is_some()`).
    pub(crate) fn finalize_undo_restore(&mut self, extras: UndoExtras) {
        self.restore_external_instruments(&extras);
        self.compose.derived_clips = extras.compose_derived_clips;
        self.compose.next_derived_clip_id = extras.compose_next_derived_clip_id;
        self.compose.vocal_audio.clip_lyrics = extras.vocal_clip_lyrics;
        self.reference.restore_undo(extras.reference);
        self.chord_track = extras.chord_track;
        restore_arrangements(&mut self.compose, &extras.compose_arrangements);
        self.apply_freeze_restore(extras.track_freeze);
        self.apply_clip_fade_gain_restore(&extras.clip_fade_gain);
    }

    /// Re-apply snapshotted clip fade/gain to the GUI mirror and re-sync the
    /// engine. Used by both restore paths (the slow `finalize_undo_restore`
    /// and the fast `try_diff_replay`). For each clip still present, the
    /// stored fade/gain is written to [`crate::state::ClipState`] and pushed
    /// to the engine via `SetClipFade` / `SetClipGain` — the same commands
    /// the live edits use, so undo/redo and direct editing share one code
    /// path. Clips absent from the map (or absent from the project) are
    /// left untouched. Reads only app-side state — no engine read-getters.
    pub(crate) fn apply_clip_fade_gain_restore(&mut self, map: &HashMap<ClipId, ClipFadeGain>) {
        for clip in self.clips.iter_mut() {
            let Some(fg) = map.get(&clip.id) else {
                continue;
            };
            // Skip the engine round-trip when nothing changed, so a restore
            // that didn't touch this clip stays quiet.
            let unchanged = clip.fade_in_frames == fg.fade_in_frames
                && clip.fade_in_curve == fg.fade_in_curve
                && clip.fade_out_frames == fg.fade_out_frames
                && clip.fade_out_curve == fg.fade_out_curve
                && clip.gain_db == fg.gain_db;
            clip.fade_in_frames = fg.fade_in_frames;
            clip.fade_in_curve = fg.fade_in_curve;
            clip.fade_out_frames = fg.fade_out_frames;
            clip.fade_out_curve = fg.fade_out_curve;
            clip.gain_db = fg.gain_db;
            if unchanged {
                continue;
            }
            let _ = self.engine.send(AudioCommand::SetClipFade {
                clip_id: clip.id,
                fade_in_frames: fg.fade_in_frames,
                fade_in_curve: fg.fade_in_curve,
                fade_out_frames: fg.fade_out_frames,
                fade_out_curve: fg.fade_out_curve,
            });
            let _ = self.engine.send(AudioCommand::SetClipGain {
                clip_id: clip.id,
                gain_db: fg.gain_db,
            });
        }
    }

    /// Drive the engine + GUI external-instrument state back to `extras`.
    /// Shared by both undo restore paths (the diff replay in
    /// `try_diff_replay` and the full `AllCleared` replay via
    /// `finalize_undo_restore`).
    ///
    /// Clears tracks that are no longer external, then (re-)asserts every
    /// target config via `SetExternalInstrument` — idempotent on the engine
    /// and, unlike a patch send, it never re-fires MIDI to the synth. The
    /// runtime device-offline flags are preserved for tracks that stay
    /// external (live hardware status survives an undo); a track returning to
    /// external mode starts online and is re-checked on the next ping.
    pub(crate) fn restore_external_instruments(&mut self, extras: &UndoExtras) {
        // Drop external mode from tracks absent in the target snapshot.
        let stale: Vec<TrackId> = self
            .external_instruments
            .keys()
            .copied()
            .filter(|id| !extras.external_instruments.contains_key(id))
            .collect();
        for id in stale {
            self.external_instruments.remove(&id);
            let _ = self
                .engine
                .send(AudioCommand::ClearExternalInstrument { track_id: id });
        }
        // Re-assert every target config, keeping live offline flags.
        for (id, config) in &extras.external_instruments {
            let _ = self.engine.send(AudioCommand::SetExternalInstrument {
                config: *config,
            });
            let state = self
                .external_instruments
                .entry(*id)
                .or_insert_with(|| crate::state::ExternalInstrumentState::new(*id));
            state.apply_config(config);
        }
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

impl crate::Resonance {
    /// Run the undo-history side effects for a single message dispatch.
    /// Classifies the message, marks the project dirty when appropriate,
    /// and captures a pre-dispatch snapshot for the Record / RecordCoalesced
    /// / Begin actions. Returns `true` when the caller must call
    /// `self.undo.commit()` after dispatch — i.e. when the message is a
    /// gesture-end that closes a transaction opened by an earlier `Begin`.
    pub(crate) fn record_undo(&mut self, message: &crate::message::Message) -> bool {
        let action = classify(message);
        let commit_after = matches!(action, UndoAction::Commit);

        // Mark the project dirty on any state-changing action. This
        // mirrors the undo classification: any action that warrants an
        // undo entry (Record, RecordCoalesced, Begin, Commit) means the
        // project has diverged from the last saved version. The dirty
        // flag is cleared on ProjectSaved(Ok) and on project load.
        if !matches!(action, UndoAction::Skip) {
            self.dirty = true;
        }

        // Skip every history-mutating branch when the app isn't in a
        // state where a snapshot could be restored (no active project,
        // no saved path, mid-restore). Commit still runs on gesture end
        // even if recording was blocked — it'll be a no-op because
        // `begin` was also blocked, so there's no pending transaction.
        if self.can_record_undo() {
            match action {
                UndoAction::Skip | UndoAction::Commit => {}
                UndoAction::Record => {
                    let snap = self.snapshot_for_undo();
                    self.undo.record(snap);
                }
                UndoAction::RecordCoalesced(key) => {
                    let snap = self.snapshot_for_undo();
                    self.undo.record_coalesced(snap, key);
                }
                UndoAction::Begin => {
                    let snap = self.snapshot_for_undo();
                    self.undo.begin(snap);
                }
            }
        }

        commit_after
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
    use crate::reference::ReferenceMessage;

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
        Message::Export(_) => UndoAction::Skip,
        // The import modal is transient dialog state until the actual
        // import lands (a follow-up todo, doc #158); none of its
        // interactions mutate the project yet, so nothing to record.
        Message::Import(_) => UndoAction::Skip,
        // Missing-file relink (doc #175, todo #600). Opening the OS
        // picker, its cancel results, and starting the background import
        // are transient — they mutate no project state. Only the applied
        // outcome (`Imported(Ok)`) clears the missing flag, refreshes the
        // asset's source provenance, and reloads its clips, so that one
        // records a pre-relink snapshot to make the relink reversible.
        // `Imported(Err)` only sets a transient error string.
        Message::Relink(RelinkMessage::Imported(Ok(_))) => UndoAction::Record,
        Message::Relink(_) => UndoAction::Skip,
        // Media-browser navigation, filtering, favourite / recent, and
        // audition preview are all transient session UI state (doc #175) —
        // never undoable and never in the project file, same rule as the
        // collapse toggles. Favourites / recent persist to user settings
        // (not the project); the engine's preview transport is outside
        // undo entirely.
        Message::Browser(_) => UndoAction::Skip,
        // Drag-to-timeline placement preview (doc #175, todo #605) is pure
        // transient UI: the drag pill, lit lane, ghost clip and tooltip are
        // never undoable and never in the project file. The one durable
        // effect — the drop — re-dispatches a `Pool(ImportAndPlace)`, which
        // records its own single undo entry via the arm below.
        Message::Drag(_) => UndoAction::Skip,
        // Audio import + placement (doc #175, todo #598) is one undoable
        // action. Recording here — before the import command is even sent —
        // captures the pre-import project (no pool asset, no placed clip, no
        // spawned track); the asset lands asynchronously and mutates state
        // via the engine-event path, which never records undo. So one undo
        // of this single snapshot removes the whole import + placement. Both
        // the pool-only and place variants are reversible (a pool asset
        // rides the `ProjectFile` snapshot just like a clip does).
        Message::Pool(_) => UndoAction::Record,
        Message::GlobalTrack(GlobalTrackMessage::SelectEvent(_)) => UndoAction::Skip,
        Message::GlobalTrack(GlobalTrackMessage::StartTempoDrag(_)) => UndoAction::Begin,
        Message::GlobalTrack(GlobalTrackMessage::EndTempoDrag) => UndoAction::Commit,
        Message::GlobalTrack(GlobalTrackMessage::UpdateTempoEvent { .. }) => UndoAction::Skip,
        Message::GlobalTrack(_) => UndoAction::Record,

        // Every chord-track edit is a discrete action (no drag gestures
        // reach the update layer — todo #441), so each records one entry.
        Message::ChordTrack(_) => UndoAction::Record,

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

        Message::Marker(m) => match m {
            // Mutating edits: record an undo entry capturing the
            // pre-edit marker set (markers ride the ProjectFile
            // snapshot/replay path). `LoopToRegion` mutates the loop
            // range, matching `ToggleLoop`'s classification.
            MarkerMessage::AddAtPlayhead
            | MarkerMessage::Rename(_, _)
            | MarkerMessage::Recolor(_, _)
            | MarkerMessage::Delete(_)
            | MarkerMessage::LoopToRegion(_)
            | MarkerMessage::SeedFromSections => UndoAction::Record,
            // Drag gestures: a marker move or a region-edge resize fires
            // one message per pointer step, so coalesce each gesture into a
            // single undo entry keyed by marker id (mirrors fader / knob
            // bursts). A one-off convert-to-region / point still records a
            // lone entry — nothing to coalesce it with.
            MarkerMessage::MoveStart(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::MarkerMove(*id))
            }
            MarkerMessage::SetRegionEnd(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::MarkerResize(*id))
            }
            // Navigation only — moves the playhead / starts playback,
            // no project mutation, mirroring `SeekToSample` / `Play`.
            MarkerMessage::JumpToNext
            | MarkerMessage::JumpToPrev
            | MarkerMessage::JumpTo(_)
            | MarkerMessage::PlayFromMarker(_) => UndoAction::Skip,
        },

        // Committing an inline rename edits the persisted marker name, so it
        // records an undo entry exactly like `MarkerMessage::Rename` above.
        Message::MarkerUi(MarkerUiMessage::CommitRename) => UndoAction::Record,
        // Every other marker-interaction message (selection, menu open/close,
        // rename begin/change/cancel) is pure view state — never undoable.
        Message::MarkerUi(_) => UndoAction::Skip,

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

        Message::ExternalInstrument(e) => match e {
            // Runtime-only: re-checking devices / re-scanning hardware
            // mutates no project state.
            ExternalInstrumentMessage::CheckDevices(_)
            | ExternalInstrumentMessage::RescanDevices => UndoAction::Skip,
            // Every config change (enable/disable, route, patch, latency,
            // monitor, arm) is a user-meaningful, reversible edit.
            _ => UndoAction::Record,
        },

        Message::Bus(b) => match b {
            BusMessage::SetBusVolume(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::BusVolume(*id))
            }
            BusMessage::SetBusPan(id, _) => UndoAction::RecordCoalesced(CoalesceKey::BusPan(*id)),
            _ => UndoAction::Record,
        },

        // Aux-send edits. A level drag coalesces into one entry per
        // gesture (like the volume/pan faders); every other send action is
        // a discrete, atomic edit. End-to-end *restoration* of sends on
        // undo is completed by the persistence slice (ba todo #482), which
        // teaches the snapshot/replay path about the send graph; here we
        // only classify the bookkeeping (dirty-mark + redo-clear).
        Message::Mixer(m) => match m {
            MixerMessage::SetSendLevel(send_id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::SendLevel(*send_id))
            }
            MixerMessage::AddSend { .. }
            | MixerMessage::RemoveSend(_)
            | MixerMessage::SetSendDest(_, _)
            | MixerMessage::ToggleSendPreFader(_)
            | MixerMessage::ToggleSendEnabled(_)
            | MixerMessage::SetBusReturnRole(_, _)
            | MixerMessage::CreateReturnFromSend { .. } => UndoAction::Record,
        },
        // Freeze edits. Freeze / unfreeze / refreeze / batch-freeze are
        // discrete, atomic transitions worth an undo entry; the rendered
        // cache is deliberately excluded from history (see `UndoExtras` and
        // `apply_freeze_restore`). Cancelling an in-flight render is a
        // transient abort, not a project mutation — skip it.
        Message::Freeze(f) => match f {
            FreezeMessage::CancelFreeze => UndoAction::Skip,
            FreezeMessage::FreezeTrack(_)
            | FreezeMessage::UnfreezeTrack(_)
            | FreezeMessage::RefreezeTrack(_)
            | FreezeMessage::FreezeSelectedTracks
            | FreezeMessage::FreezeAllTracks => UndoAction::Record,
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
            ClipMessage::StartClipFadeDrag { .. } | ClipMessage::StartClipGainDrag { .. } => {
                UndoAction::Begin
            }
            ClipMessage::EndClipDrag | ClipMessage::EndClipTrim => UndoAction::Commit,
            ClipMessage::EndClipFadeDrag | ClipMessage::EndClipGainDrag => UndoAction::Commit,
            ClipMessage::UpdateClipDrag(_, _) | ClipMessage::UpdateClipTrim(_) => UndoAction::Skip,
            ClipMessage::UpdateClipFadeDrag(_) | ClipMessage::UpdateClipGainDrag(_) => {
                UndoAction::Skip
            }
            ClipMessage::DeleteClip(_) => UndoAction::Record,
            // Inspector flyout edits (todo #319): each is one discrete,
            // atomic edit — record a single undo entry per change, like the
            // numeric edits elsewhere. The drag gestures above coalesce via
            // Begin/Commit; these don't.
            ClipMessage::SetClipFadeInMs { .. }
            | ClipMessage::SetClipFadeOutMs { .. }
            | ClipMessage::SetClipGainDb { .. }
            | ClipMessage::SetClipFadeInCurve { .. }
            | ClipMessage::SetClipFadeOutCurve { .. }
            | ClipMessage::ResetClipFadeGain { .. } => UndoAction::Record,
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
            | MidiEditorMessage::RemoveSelectedNotes { .. }
            | MidiEditorMessage::MoveNote { .. }
            | MidiEditorMessage::ResizeNote { .. }
            | MidiEditorMessage::ToggleSlur { .. }
            // Bulk timing edits (doc #163): each rewrites the clip's note
            // array, so the pre-dispatch snapshot of the prior notes is
            // the single undo step. Humanize draws its seed in the handler,
            // so re-doing rolls a new feel — undo still restores the exact
            // prior notes via the snapshot, which is what matters.
            | MidiEditorMessage::Quantize { .. }
            | MidiEditorMessage::Humanize { .. }
            | MidiEditorMessage::ApplyGroove { .. } => UndoAction::Record,
            // Groove *extraction* reads the clip and produces a template;
            // it never mutates the notes, so there's nothing to undo here.
            // Library persistence/undo is a separate slice (#395).
            MidiEditorMessage::ExtractGroove { .. } => UndoAction::Skip,
            MidiEditorMessage::OpenMidiEditor(_)
            | MidiEditorMessage::OpenSelectedMidiClip
            | MidiEditorMessage::CloseMidiEditor
            | MidiEditorMessage::SelectNote { .. }
            | MidiEditorMessage::ToggleNoteSelection { .. }
            | MidiEditorMessage::SelectNotesInRect { .. }
            | MidiEditorMessage::SelectAllNotes
            | MidiEditorMessage::ClearNoteSelection
            | MidiEditorMessage::PreviewNote(_, _)
            | MidiEditorMessage::StopPreview(_, _)
            | MidiEditorMessage::ScrollY(_)
            // Quantize-panel control edits (todo #392) just mutate view
            // state — the actual note edit is the `Quantize` message above.
            | MidiEditorMessage::SetQuantizeGrid(_)
            | MidiEditorMessage::SetQuantizeStrength(_)
            | MidiEditorMessage::SetQuantizeSwing(_)
            | MidiEditorMessage::SetQuantizeMode(_)
            | MidiEditorMessage::SetQuantizeEnds(_)
            | MidiEditorMessage::SetQuantizeIterative(_)
            // Humanize-panel control edits (todo #393) likewise just mutate
            // view state — the note edit is the `Humanize` message above.
            | MidiEditorMessage::SetHumanizeTiming(_)
            | MidiEditorMessage::SetHumanizeVelocity(_)
            // Groove-panel control edits (todo #394) just mutate view state —
            // the note edit is the `ApplyGroove` message; extract is read-only.
            | MidiEditorMessage::SetGrooveName(_)
            | MidiEditorMessage::SetGrooveSelection(_)
            | MidiEditorMessage::SetGrooveStrength(_) => UndoAction::Skip,
        },

        // Pitch-editor open/close is editor lifecycle + an analysis
        // request (a read-only engine query whose result is cached, not
        // user-authored project data) — never an undoable edit, mirroring
        // the MIDI editor open/close above.
        Message::VocalTuning(_) => UndoAction::Skip,

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
            | ComposeMessage::ToggleRailPanel(_)
            | ComposeMessage::ToggleWorkspaceGroup(_)
            | ComposeMessage::ExpandTrack { .. }
            | ComposeMessage::CollapseTrack
            | ComposeMessage::ExpandedScrollX(_)
            | ComposeMessage::ExpandedScrollY(_)
            | ComposeMessage::ExpandedZoomY(_) => UndoAction::Skip,

            // Everything else in Compose mutates project state.
            _ => UndoAction::Record,
        },

        // Reference-track (A/B). Only the content-changing actions named
        // in the design (load / remove / set-active / loudness-match /
        // trim) are reversible; the trim drag coalesces. The monitoring
        // toggles, markers, scrub, and error dismissal are transient.
        Message::Reference(rm) => match rm {
            ReferenceMessage::LoadRequested(_)
            // A picked file ends in the same load path as a drag-drop, so
            // a successful pick is just as reversible; a cancelled pick
            // (`None`) changes nothing.
            | ReferenceMessage::FilePicked(Some(_))
            | ReferenceMessage::Remove(_)
            | ReferenceMessage::SetActive(_)
            | ReferenceMessage::ToggleLoudnessMatch => UndoAction::Record,
            ReferenceMessage::TrimChanged(_) => {
                UndoAction::RecordCoalesced(CoalesceKey::ReferenceTrim)
            }
            // Opening the picker and a cancelled pick are pure UI / no-ops;
            // the monitoring toggles, markers, scrub, and error dismissal
            // are transient.
            ReferenceMessage::PickFile
            | ReferenceMessage::FilePicked(None)
            | ReferenceMessage::ToggleAbSource
            | ReferenceMessage::SetAbSource(_)
            | ReferenceMessage::MomentaryAudition(_)
            | ReferenceMessage::AddMarker { .. }
            | ReferenceMessage::RemoveMarker { .. }
            | ReferenceMessage::Scrub { .. }
            | ReferenceMessage::ToggleLoopToMix
            | ReferenceMessage::DismissError => UndoAction::Skip,
        },
    }
}
