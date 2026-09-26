//! Undo snapshot data model and the `Resonance` methods that build, restore,
//! and apply snapshots.
//!
//! Three kinds of types live here:
//! - The snapshot itself (`UndoSnapshot`: a `LoadedProject`) and the
//!   `UndoExtras` it still carries beside the `ProjectFile` (ARCH-01 A1-2
//!   folds those into the file one at a time).
//! - The `CoalesceKey` discriminator used by the history stack to merge a
//!   burst of fader/knob messages into one undo entry.
//! - The `impl crate::Resonance` blocks for building a snapshot
//!   (`snapshot_for_undo`), checking preconditions, driving undo/redo, and
//!   applying a restored snapshot back to the live engine.

use std::collections::HashMap;
use std::sync::Arc;

use resonance_audio::types::{AudioCommand, ClipId, MidiNote, PluginInstanceId};
use resonance_common::{AutomationLane, AutomationTarget};

use crate::project::LoadedProject;
use resonance_audio::types::TrackId;

/// State an undo snapshot carries beside its `ProjectFile`, re-applied
/// after the replay by both restore paths (`try_diff_replay` and
/// `finalize_undo_restore`).
///
/// Most of it is *already* in the `ProjectFile` too, and the full replay
/// restores it from there; the diff replay still reads it from here. It
/// is being folded away field by field (ARCH-01 A1-2): a field leaves
/// once both paths restore it from the file, guarded by
/// `tests/io/undo_snapshot_fixed_point.rs`. Clip fade/gain, the drum
/// arrangements, the chord track and the external-instrument config
/// already went — `ProjectClip`, `ProjectSectionDefinition::arrangement`,
/// `ProjectFile::chord_track` and `ProjectTrack::external_instrument`
/// carry them.
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
    /// App-side parameter-automation lanes, one per target. Also
    /// persisted as `ProjectFile::automation_lanes`, which the full
    /// replay reads; the diff replay reconciles the engine to this copy
    /// via `restore_automation_lanes`.
    pub automation_lanes: HashMap<AutomationTarget, AutomationLane>,
    /// Reference-track (A/B) content. The durable facts are also in
    /// `ProjectFile::references` / `reference_settings`, but the live
    /// entries (engine ids, analysis status) are reapplied from here after
    /// the replay on both paths, without yanking the monitor around.
    pub reference: crate::reference::ReferenceUndo,
    /// Per-track freeze status at snapshot time. The rendered cache is not
    /// part of undo history, so on restore
    /// [`crate::Resonance::apply_freeze_restore`] detaches + deletes the
    /// cache of any track that is no longer frozen and downgrades a
    /// re-frozen track whose cache file is gone to stale.
    pub track_freeze: HashMap<TrackId, crate::state::FreezeStatus>,
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
    /// State re-applied after the replay; see [`UndoExtras`].
    pub extras: UndoExtras,
}

impl UndoSnapshot {
    /// True when `self` and `other` describe the same undoable state — the
    /// check that tells a gesture that edited something from a click that
    /// moved nothing (code review STATE-07). Compares every captured part:
    /// the project file by its derived `PartialEq` (ARCH-01 A-8 — the whole
    /// tree derives it now, so this is a plain struct compare; map-valued
    /// fields compare order-independently the same way the old
    /// `serde_json` compare did through its key-sorted objects), notes
    /// field by field, and the extras directly.
    pub(crate) fn same_state(&self, other: &UndoSnapshot) -> bool {
        let notes_equal = self.project.midi_notes.len() == other.project.midi_notes.len()
            && self.project.midi_notes.iter().all(|(id, notes)| {
                other
                    .project
                    .midi_notes
                    .get(id)
                    .is_some_and(|o| crate::update::project_io::replay_diff::midi_notes_equal(notes, o))
            });
        notes_equal
            && extras_equal(&self.extras, &other.extras)
            && self.project.file == other.project.file
    }
}

fn extras_equal(a: &UndoExtras, b: &UndoExtras) -> bool {
    a.compose_derived_clips == b.compose_derived_clips
        && a.compose_next_derived_clip_id == b.compose_next_derived_clip_id
        && a.vocal_clip_lyrics == b.vocal_clip_lyrics
        && a.automation_lanes == b.automation_lanes
        && a.reference.entries == b.reference.entries
        && a.reference.active_id == b.reference.active_id
        && a.reference.loudness_match == b.reference.loudness_match
        && a.reference.offset_db.to_bits() == b.reference.offset_db.to_bits()
        && a.reference.trim_db.to_bits() == b.reference.trim_db.to_bits()
        && a.track_freeze == b.track_freeze
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
    /// A group macro level-trim slider drag, keyed by group id (epic #36).
    GroupMacroLevel(u64),
    /// Everything one recording session lands — every armed track's
    /// `RecordingFinished`, each cycle-record `TakeCaptured`, the MIDI
    /// clips a live recording opens — so the whole take is one entry.
    /// `RecordingStarted` breaks the run, so each session is its own.
    Recording,
}

// -------------------------------------------------------------------------
// Resonance snapshot-building and restore methods
// -------------------------------------------------------------------------

impl crate::Resonance {
    /// Build an undo snapshot of the current declarative project state.
    ///
    /// Parameter values come from live GUI state (via
    /// `build_project_file`), so they are always exact. Opaque CLAP state
    /// blobs come from `plugin_state_cache`, which refreshes at natural
    /// resting points — plugin add, editor close, project save — and is
    /// therefore slightly stale between those points.
    ///
    /// Also asks the engine to persist every audio clip's
    /// `audio/clip_<id>.wav` (code review FU-V5b): the snapshot names a
    /// clip's audio only by that file, which the slow-path restore
    /// reloads, and which otherwise exists only after a save. Sent here —
    /// before the edit's own commands, which the engine runs after it —
    /// so a clip the edit removes is persisted while it still exists.
    pub(crate) fn snapshot_for_undo(&self) -> UndoSnapshot {
        if !self.clips.is_empty() && self.can_record_undo() {
            let _ = self.engine.send(AudioCommand::PersistClipWavs);
        }
        let file = crate::update::build_project_file(self);
        let midi_notes: HashMap<ClipId, Vec<MidiNote>> = self
            .midi_clips
            .iter()
            .map(|mc| (mc.id, mc.notes.clone()))
            .collect();
        let extras = self.undo_extras();
        // Only snapshot blobs for plugins that currently exist — stale
        // entries for removed plugins would bloat the snapshot and are
        // never consumed anyway. Each entry is a refcount bump on the
        // cache's `Arc`, not a copy of the blob (ARCH-09 A9-2).
        let mut plugin_states: HashMap<PluginInstanceId, Arc<[u8]>> = HashMap::new();
        let collect = |slots: &[crate::state::PluginSlotState],
                       out: &mut HashMap<PluginInstanceId, Arc<[u8]>>,
                       cache: &HashMap<PluginInstanceId, Arc<[u8]>>| {
            for slot in slots {
                if let Some(blob) = cache.get(&slot.instance_id) {
                    out.insert(slot.instance_id, Arc::clone(blob));
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

    /// Whether the live state differs from `before`, the snapshot a
    /// gesture opened with — the question `commit_undo_gesture` asks
    /// (code review STATE-07). Answered without building a second full
    /// snapshot: the notes are compared in place against `midi_clips`,
    /// the extras against a fresh (small) capture, and only the
    /// `ProjectFile` is rebuilt for the struct comparison — no note
    /// vectors, plugin blobs or project path are copied. Same verdict as
    /// `before.same_state(&self.snapshot_for_undo())`, cheaper.
    pub(crate) fn gesture_changed_since(&self, before: &UndoSnapshot) -> bool {
        let notes_equal = before.project.midi_notes.len() == self.midi_clips.len()
            && self.midi_clips.iter().all(|mc| {
                before.project.midi_notes.get(&mc.id).is_some_and(|o| {
                    crate::update::project_io::replay_diff::midi_notes_equal(&mc.notes, o)
                })
            });
        if !notes_equal || !extras_equal(&before.extras, &self.undo_extras()) {
            return true;
        }
        let file = crate::update::build_project_file(self);
        before.project.file != file
    }

    /// The runtime-only state an undo snapshot carries beside its
    /// `ProjectFile`; see [`UndoExtras`].
    fn undo_extras(&self) -> UndoExtras {
        UndoExtras {
            compose_derived_clips: self.compose.derived_clips.clone(),
            compose_next_derived_clip_id: self.compose.next_derived_clip_id,
            vocal_clip_lyrics: self.compose.vocal_audio.clip_lyrics.clone(),
            automation_lanes: self.automation.lanes.clone(),
            reference: self.reference.undo_snapshot(),
            track_freeze: self.freeze.statuses.clone(),
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
    ///
    /// # Take lanes are restored by the replay, not re-asserted after it
    ///
    /// Both paths end in `replay_take_groups`, which sends
    /// `AudioCommand::RestoreTakeGroups` — on the fast path from
    /// `apply_take_groups` inside [`crate::update::try_diff_replay`], on
    /// the slow path from `replay_loaded_project`, which the `AllCleared`
    /// handler runs immediately before `finalize_undo_restore`. That
    /// command replaces the engine's take-group store wholesale (comp and
    /// active take included, since both ride the `TakeGroup`) and
    /// republishes the comp table, so the engine plays and bounces the
    /// restored lanes without touching the transport.
    ///
    /// #411 additionally re-sent every mirrored group's `SetTakeComp` +
    /// `SetActiveTake` here (`resync_take_comps`), because at the time
    /// nothing else told the engine about a restore at all. Todo #1394
    /// landed `RestoreTakeGroups` and that resync became a strict subset
    /// of it: same two values, read from the same `take_groups` mirror,
    /// sent one command later. It was removed rather than kept as belt
    /// and braces (ba todo #1399) because it was also the *weaker* of the
    /// two — it silently no-ops for a group the engine does not hold, and
    /// it cannot bring back a take an undo just restored, which are
    /// exactly the cases `RestoreTakeGroups` exists to cover — and
    /// because it was not free: those two commands echo
    /// `TakeCompChanged` / `ActiveTakeChanged` per group on every history
    /// step, and `RestoreTakeGroups` was deliberately made silent so a
    /// restore does not come up dirty.
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

    /// Apply the extras captured in the snapshot. Called from the
    /// `AllCleared` engine-event handler immediately after
    /// `replay_loaded_project` runs, only when the pending load came
    /// from an undo/redo (distinguished by `pending_undo_extras.is_some()`).
    /// Clip fade/gain, the drum arrangements, the chord track and the
    /// external-instrument config need nothing here: the replay restores
    /// them from the snapshot's `ProjectFile`.
    pub(crate) fn finalize_undo_restore(&mut self, extras: UndoExtras) {
        self.restore_automation_lanes(&extras.automation_lanes);
        // `ClearAll` wiped any clip whose echo was still pending at
        // snapshot time; nothing will re-create it.
        self.restore_derived_clips(
            extras.compose_derived_clips,
            extras.compose_next_derived_clip_id,
            false,
        );
        self.compose.vocal_audio.clip_lyrics = extras.vocal_clip_lyrics;
        self.reference.restore_undo(extras.reference);
        self.apply_freeze_restore(extras.track_freeze);
        // Take lanes are *not* reconciled here. `replay_loaded_project`
        // runs immediately before this and ends in `replay_take_groups`,
        // which sends `RestoreTakeGroups` — see the note on the fast path
        // in `begin_restore_from_snapshot`.
    }

    /// Restore the compose section→clip map and its id counter from a
    /// snapshot — the one rule both restore paths share (FU-H2a).
    ///
    /// The map is the snapshot's, not a rebuild from the mirror: a
    /// snapshot taken while a re-derived clip's `MidiClipCreated` echo was
    /// in flight holds that clip's entry but not the clip, and on the
    /// diff replay (`echoes_in_flight`) the engine still has the clip and
    /// the echo will land — dropping the entry would orphan it. After a
    /// full replay the engine holds only the replayed clips, so there an
    /// entry whose clip is not mirrored can never be satisfied and is
    /// dropped (left in, it would also suspend the UPD-05 freeze check on
    /// its track forever, see `revalidate_frozen_content`).
    pub(crate) fn restore_derived_clips(
        &mut self,
        mut derived: HashMap<(u64, u64, TrackId), ClipId>,
        next_derived_clip_id: u64,
        echoes_in_flight: bool,
    ) {
        if !echoes_in_flight {
            derived.retain(|_, id| self.midi_clips.iter().any(|mc| mc.id == *id));
        }
        self.compose.derived_clips = derived;
        self.compose.next_derived_clip_id = next_derived_clip_id;
        self.compose
            .reserve_derived_clip_ids(self.midi_clips.iter().map(|mc| mc.id));
    }

    /// Drive the engine + GUI external-instrument state back to `target`'s
    /// `ProjectTrack::external_instrument`s — the diff replay's
    /// (`try_diff_replay`) restore. The full replay restores the same
    /// state per track in `replay_track`, after `ClearAll` wiped it.
    ///
    /// Clears tracks that are no longer external, then (re-)asserts every
    /// target config via `SetExternalInstrument` — idempotent on the engine
    /// and, unlike a patch send, it never re-fires MIDI to the synth. The
    /// selected device preset (epic #40) is restored too: the id is re-applied
    /// to the GUI state and the resolved params re-sent via
    /// `SetTrackDeviceParams`, so device selection reverses with the rest of
    /// the config. The runtime device-offline flags are preserved for tracks
    /// that stay external (live hardware status survives an undo); a track
    /// returning to external mode starts online and is re-checked on the next
    /// ping.
    pub(crate) fn restore_external_instruments(&mut self, target: &crate::project::ProjectFile) {
        let externals: HashMap<TrackId, &crate::project::ProjectExternalInstrument> = target
            .tracks
            .iter()
            .filter_map(|pt| pt.external_instrument.as_ref().map(|ext| (pt.id, ext)))
            .collect();
        // Drop external mode from tracks absent in the target snapshot. Clear
        // their engine device-param map too, so a track leaving external mode
        // doesn't leave stale bindings behind on the engine side.
        let stale: Vec<TrackId> = self
            .external_instruments
            .keys()
            .copied()
            .filter(|id| !externals.contains_key(id))
            .collect();
        for id in stale {
            self.external_instruments.remove(&id);
            let _ = self
                .engine
                .send(AudioCommand::ClearExternalInstrument { track_id: id });
            let _ = self.engine.send(AudioCommand::SetTrackDeviceParams {
                track_id: id,
                params: Vec::new(),
            });
        }
        // Re-assert every target config, keeping live offline flags.
        for (&id, ext) in &externals {
            let config = ext.config(id);
            let _ = self
                .engine
                .send(AudioCommand::SetExternalInstrument { config });
            // Restore the selected device preset and re-send its params
            // (an empty map when none is selected).
            let params = ext.device_params(&self.device_registry);
            let _ = self.engine.send(AudioCommand::SetTrackDeviceParams {
                track_id: id,
                params,
            });
            let state = self
                .external_instruments
                .entry(id)
                .or_insert_with(|| crate::state::ExternalInstrumentState::new(id));
            state.apply_config(&config);
            state.device_id = ext.device_id.clone();
        }
    }

    /// Reconcile the engine's automation lanes to `target` and overwrite
    /// the app-side mirror to match. Shared by both undo/redo restore
    /// paths (the structure-preserving diff replay and the full
    /// clear-and-replay), and correct for either because the engine's
    /// lane set always equals the current mirror at call time: the diff
    /// path never touches lanes, and `ClearAll` deliberately leaves the
    /// engine's automation lanes intact (they are re-keyed per target,
    /// not by track/clip identity).
    ///
    /// Lanes present now but absent from `target` are cleared; lanes that
    /// are new or whose breakpoints/Read flag changed are re-sent
    /// whole-lane. Transient live-value tints for dropped targets are
    /// discarded so a stale fader/knob tint can't outlive its lane.
    pub(crate) fn restore_automation_lanes(
        &mut self,
        target: &HashMap<AutomationTarget, AutomationLane>,
    ) {
        let stale: Vec<AutomationTarget> = self
            .automation
            .lanes
            .keys()
            .filter(|t| !target.contains_key(t))
            .cloned()
            .collect();
        for t in stale {
            let _ = self.engine.send(AudioCommand::ClearAutomationLane { target: t });
        }
        for (t, lane) in target {
            if self.automation.lanes.get(t) != Some(lane) {
                let _ = self
                    .engine
                    .send(AudioCommand::SetAutomationLane { lane: lane.clone() });
            }
        }
        self.automation.lanes = target.clone();
        self.automation.bump_allocator_past_lanes();
        self.automation
            .live_values
            .retain(|t, _| target.contains_key(t));
    }

    /// Attempt to undo. No-ops (returning false) if the history is empty
    /// or an in-flight operation blocks undo/redo. On success the current
    /// state is pushed onto the redo stack before the snapshot is
    /// restored.
    pub(crate) fn try_undo(&mut self) -> Option<String> {
        if !self.can_undo_redo_now() || !self.undo.can_undo() {
            return None;
        }
        let (snapshot, label) = self.undo.pop_undo()?;
        // An import whose entry this undo pops must not place its clip
        // when the file lands later (code review UPD-04).
        self.pool_import.drop_undone(self.undo.undo_len());
        let current = self.snapshot_for_undo();
        // The action just undone is what a redo would re-apply, so its
        // label travels with the state pushed onto the redo stack.
        self.undo.push_redo(current, label.clone());
        self.begin_restore_from_snapshot(snapshot);
        // An undo changes the song like any committed edit — remote
        // control clients detect it through the revision counter
        // (doc #265, todo #1147).
        self.revision = self.revision.wrapping_add(1);
        Some(label)
    }

    /// Symmetric counterpart to `try_undo`.
    pub(crate) fn try_redo(&mut self) -> Option<String> {
        if !self.can_undo_redo_now() || !self.undo.can_redo() {
            return None;
        }
        let (snapshot, label) = self.undo.pop_redo()?;
        let current = self.snapshot_for_undo();
        self.undo.push_undo(current, label.clone());
        self.begin_restore_from_snapshot(snapshot);
        // Symmetric to `try_undo`: a redo is a committed edit for remote
        // revision-tracking purposes.
        self.revision = self.revision.wrapping_add(1);
        Some(label)
    }
}
