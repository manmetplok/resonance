//! Undo snapshot data model and the `Resonance` methods that build, restore,
//! and apply snapshots.
//!
//! Three kinds of types live here:
//! - The snapshot itself (`UndoSnapshot`: a `LoadedProject`, nothing
//!   beside it — ARCH-01 A1-2 folded every former `UndoExtras` field into
//!   the `ProjectFile`, and A-7 deleted the type).
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

/// One point in the undo/redo history. Wraps the `LoadedProject` shape
/// a disk load restores from, so both share the `Reconcile` driver. Everything undoable travels in the
/// `ProjectFile` (+ notes + plugin blobs); there is no side-car state
/// (ARCH-01 A1-2 folded it into the file, guarded by
/// `tests/io/undo_snapshot_fixed_point.rs`; the derived-clip id counter is
/// session state, not undo state, A-6).
#[derive(Debug, Clone)]
pub struct UndoSnapshot {
    /// Declarative project state in the exact shape the replay path
    /// expects. `plugin_states` is populated from
    /// `Resonance::plugin_mirror.state_cache` at snapshot time; missing entries
    /// cause the restore path to reinstantiate the plugin with default
    /// internal state and rely on the replayed parameter values.
    pub project: LoadedProject,
    /// Plugin states that arrive after the snapshot was taken: the full
    /// state a preset load saved (under the plugin's lock) just before it
    /// replaced it, so undoing the load puts back the model / IR / user
    /// tables the preset changed, not the possibly stale cached blob. A
    /// filled entry overrides `project.plugin_states` at restore.
    pub(crate) late_plugin_states: Vec<(PluginInstanceId, LateBlob)>,
}

/// A plugin state filled in after the snapshot that holds it.
pub(crate) type LateBlob = Arc<std::sync::Mutex<Option<Arc<[u8]>>>>;

impl UndoSnapshot {
    /// A snapshot of `project` with no late plugin states.
    pub fn new(project: LoadedProject) -> Self {
        Self {
            project,
            late_plugin_states: Vec::new(),
        }
    }

    /// True when `self` and `other` describe the same undoable state — the
    /// check that tells a gesture that edited something from a click that
    /// moved nothing (code review STATE-07). Compares every captured part:
    /// the project file by its derived `PartialEq` (ARCH-01 A-8 — the whole
    /// tree derives it now, so this is a plain struct compare; map-valued
    /// fields compare order-independently the same way the old
    /// `serde_json` compare did through its key-sorted objects), and notes
    /// field by field. Nothing else is captured.
    pub(crate) fn same_state(&self, other: &UndoSnapshot) -> bool {
        let notes_equal = self.project.midi_notes.len() == other.project.midi_notes.len()
            && self.project.midi_notes.iter().all(|(id, notes)| {
                other.project.midi_notes.get(id).is_some_and(|o| {
                    // A clip nothing touched between the two snapshots
                    // shares the same `Arc` (ARCH-09 A9-3) — check that
                    // before the element-wise compare.
                    Arc::ptr_eq(notes, o)
                        || crate::update::project_io::replay_diff::midi_notes_equal(notes, o)
                })
            });
        notes_equal && self.project.file == other.project.file
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
    /// Consecutive ◀ / ▶ preset steps on one plugin: one entry for the
    /// run, back to where it started.
    PluginPresetStep(u64),
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
    /// Typing in a track's name field (lane inspector instrument panel),
    /// keyed by track so renaming two different tracks in a row is two
    /// entries but every keystroke on the same field is one (FU-A10a).
    TrackName(TrackId),
    /// Typing in the drum-group manager's rename input, keyed by group id
    /// (FU-A10a).
    DrumGroupName(u64),
    /// Dragging one of a drum group's generator knobs (density, swing,
    /// accent, humanize, fills, cycle, phase — the right-rail sliders),
    /// keyed by group id and which knob so switching knob or group breaks
    /// the run but repeated steps on the same knob merge (FU-A10a, FU-A10c).
    DrumGroupParam(u64, DrumGroupKnob),
    /// Dragging a drum pad's articulation weight slider, keyed by group id
    /// and pad index so switching pad or group breaks the run but repeated
    /// steps on the same pad merge (FU-A10c).
    DrumPadWeight(u64, usize),
    /// Dragging an external-instrument track's manual latency offset slider,
    /// keyed by track id (FU-A10c).
    ExternalLatency(TrackId),
    /// Typing in a vocal lane's lyric theme / prompt field, keyed by the
    /// lane (section definition + track) it belongs to (FU-A10a).
    VocalTheme(u64, TrackId),
    /// Typing in one draft lyric line's text field, keyed by the lane and
    /// the 1-based line number (FU-A10a).
    VocalLineText(u64, TrackId, u8),
    /// Dragging one of the lane inspector's numeric sliders (bass/melody/
    /// pad velocity & register knobs, vocal delivery params), keyed by the
    /// lane (definition + track) and which knob, so switching knob or lane
    /// breaks the run but repeated steps on the same knob merge (FU-A10b;
    /// was one undo entry per slider step).
    LaneParam(u64, TrackId, LaneParamKnob),
    /// Dragging one of the chord inspector's numeric sliders (motif
    /// complexity/leap chance, schema substitution), keyed by the section
    /// definition and which knob (FU-A10b; was one undo entry per step).
    ChordParam(u64, ChordParamKnob),
    /// Editing the bulk-lyrics text editor, keyed by the lane (definition +
    /// track). Non-edit actions (cursor moves, selection, scroll) classify
    /// `Skip` and never reach this key — see
    /// `LaneInspectorMsg::undo_action` (FU-A10b; every action, including a
    /// bare cursor move, used to record its own entry).
    VocalBulkLyrics(u64, TrackId),
}

/// Which of a drum group's right-rail generator knobs a coalesce run is
/// for — see [`CoalesceKey::DrumGroupParam`] (FU-A10a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrumGroupKnob {
    Density,
    Swing,
    Accent,
    Humanize,
    Fills,
    Cycle,
    Phase,
}

/// Which of the lane inspector's numeric sliders a coalesce run is for —
/// see [`CoalesceKey::LaneParam`] (FU-A10b). One variant per `slider(...)`
/// site in `view/compose/lane_inspector/{instrument,vocal}/*.rs`; the
/// register/note pickers next to some of these are `pick_list`s (discrete
/// picks) and stay plain `Record`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneParamKnob {
    BassVelocity,
    MelodyRestDensity,
    MelodyVelocity,
    MelodyArticulation,
    PadVelocity,
    VocalChordToneAnchor,
    VocalLeapRange,
    VocalBreath,
    VocalVibrato,
    VocalVibratoRate,
    VocalTension,
    VocalTensionVelocityAmount,
    VocalTensionContourAmount,
    VocalPortamentoMs,
    VocalArticulation,
    VocalConsonantEmphasis,
}

/// Which of the chord inspector's numeric sliders a coalesce run is for —
/// see [`CoalesceKey::ChordParam`] (FU-A10b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordParamKnob {
    MotifComplexity,
    MotifLeapChance,
    SchemaSubstitution,
}

// -------------------------------------------------------------------------
// Resonance snapshot-building and restore methods
// -------------------------------------------------------------------------

impl crate::Resonance {
    /// Build an undo snapshot of the current declarative project state.
    ///
    /// Parameter values come from live GUI state (via
    /// `build_project_file`), so they are always exact. Opaque CLAP state
    /// blobs come from `plugin_mirror.state_cache`, which refreshes at natural
    /// resting points — plugin add, editor close, project save — and is
    /// therefore slightly stale between those points.
    ///
    /// Also asks the engine to persist every audio clip's
    /// `audio/clip_<id>.wav` (code review FU-V5b): the snapshot names a
    /// clip's audio only by that file, which a restore that re-adds it
    /// reloads, and which otherwise exists only after a save. Sent here —
    /// before the edit's own commands, which the engine runs after it —
    /// so a clip the edit removes is persisted while it still exists.
    pub(crate) fn snapshot_for_undo(&self) -> UndoSnapshot {
        if !self.clips.is_empty() && self.can_record_undo() {
            let _ = self.engine.send(AudioCommand::PersistClipWavs);
        }
        let file = self.undo_project_file();
        // A refcount bump per clip, not a copy (ARCH-09 A9-3): a clip
        // `Arc::make_mut` hasn't touched since the last snapshot hands back
        // the same pointer.
        let midi_notes: HashMap<ClipId, Arc<Vec<MidiNote>>> = self
            .midi_clips
            .iter()
            .map(|mc| (mc.id, Arc::clone(&mc.notes)))
            .collect();
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
            collect(&track.plugins, &mut plugin_states, &self.plugin_mirror.state_cache);
        }
        for bus in &self.registry.busses {
            collect(&bus.plugins, &mut plugin_states, &self.plugin_mirror.state_cache);
        }
        collect(
            &self.master.plugins,
            &mut plugin_states,
            &self.plugin_mirror.state_cache,
        );

        // A preset load whose "after" state has not come back yet: this
        // snapshot (a redo's, taken by the undo) waits on it, so redo puts
        // back the state the load left rather than the stale cached one.
        let mut late_plugin_states = Vec::new();
        for owed in self.presets.pending_after.values().filter(|o| !o.superseded) {
            if plugin_states.contains_key(&owed.instance_id)
                || self.plugin_mirror.index.contains_key(&owed.instance_id)
            {
                let late = LateBlob::default();
                if let Ok(mut s) = owed.slots.lock() {
                    s.push(late.clone());
                }
                late_plugin_states.push((owed.instance_id, late));
            }
        }
        // A blob refresh still in flight (an editor kit pick): the cached
        // blob this snapshot holds predates the change, so the refresh's
        // echo fills it in (STATE2-07).
        for (&instance_id, owed) in self.plugin_mirror.owed_blobs.iter() {
            if owed.superseded || !self.plugin_mirror.index.contains_key(&instance_id) {
                continue;
            }
            let late = LateBlob::default();
            if let Ok(mut s) = owed.slots.lock() {
                s.push((owed.marks, late.clone()));
            }
            late_plugin_states.push((instance_id, late));
        }
        UndoSnapshot {
            project: LoadedProject {
                file,
                project_dir: self.io.project_path.clone().unwrap_or_default(),
                midi_notes,
                plugin_states,
            },
            late_plugin_states,
        }
    }

    /// Whether the live state differs from `before`, the snapshot a
    /// gesture opened with — the question `commit_undo_gesture` asks
    /// (code review STATE-07). Answered without building a second full
    /// snapshot: the notes are compared in place against `midi_clips`,
    /// and only the `ProjectFile` is rebuilt for the struct comparison —
    /// no note vectors, plugin blobs or project path are copied. Same
    /// verdict as `before.same_state(&self.snapshot_for_undo())`, cheaper.
    pub(crate) fn gesture_changed_since(&self, before: &UndoSnapshot) -> bool {
        let notes_equal = before.project.midi_notes.len() == self.midi_clips.len()
            && self.midi_clips.iter().all(|mc| {
                before.project.midi_notes.get(&mc.id).is_some_and(|o| {
                    Arc::ptr_eq(&mc.notes, o)
                        || crate::update::project_io::replay_diff::midi_notes_equal(&mc.notes, o)
                })
            });
        if !notes_equal {
            return true;
        }
        before.project.file != self.undo_project_file()
    }

    /// The `ProjectFile` an undo snapshot carries: `build_project_file`
    /// minus the reference A/B monitor state, which is saved with the
    /// project but is not undo state — an undo leaves it alone, and
    /// flipping the A/B switch is not an edit (ARCH-01 A-5) — plus the
    /// state-excluded plugin params, which are undo state but not file
    /// state (STATE2-03, `add_session_plugin_params`).
    fn undo_project_file(&self) -> crate::project::ProjectFile {
        let mut file = crate::update::build_project_file(self);
        file.reference_settings.clear_monitor_state();
        crate::update::project_io::add_session_plugin_params(self, &mut file);
        file
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
            && !self.session.undo.has_pending()
            && !self.freeze.any_in_flight()
    }

    /// Drive the engine and GUI back to `snapshot`, in place: the
    /// `Reconcile` driver diffs every domain against the live state's file
    /// (`reconcile_all` with `old = Some(current)`, ARCH-01 A-13) — one
    /// engine command per changed scalar, one add or remove per entity
    /// that differs, and every plugin instance that is kept stays alive.
    /// Synchronous: there is no `ClearAll` and nothing waits for
    /// `AllCleared` (the full-replay fallback went in A-13j). Playback is
    /// stopped (per v1 policy).
    ///
    /// # Take lanes are restored by the replay, not re-asserted after it
    ///
    /// The `TakeGroups` reconcile domain ends in `replay_take_groups`,
    /// which sends `AudioCommand::RestoreTakeGroups`. That command replaces the engine's take-group store wholesale (comp and
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
        let current = crate::update::build_project_file(self);
        self.restore_from_snapshot_against(&current, snapshot);
    }

    /// The body of [`Self::begin_restore_from_snapshot`], taking the live
    /// `ProjectFile` to diff against as a parameter rather than building it.
    ///
    /// `try_undo` / `try_redo` already build one — `snapshot_for_undo`, for
    /// the entry pushed onto the other stack — before calling this, and
    /// nothing mutates `self`'s project state in between (only the undo
    /// history's own bookkeeping), so that file is still exactly the live
    /// state. Calling through [`Self::begin_restore_from_snapshot`] instead
    /// would rebuild an identical one (FU-A13k: ~0.3 ms debug on the demo
    /// project, once per history step); this lets them pass the one they
    /// already have. `test_begin_restore_from_snapshot` has no such file in
    /// hand, so it goes through the building wrapper above instead.
    fn restore_from_snapshot_against(
        &mut self,
        current: &crate::project::ProjectFile,
        snapshot: UndoSnapshot,
    ) {
        use crate::update::project_io::reconcile::{reconcile_all, Origin, ReconcileCtx};

        // A step run's parked state load belongs to the sound being left:
        // sent after this restore it would override it (review: undo inside
        // the step debounce).
        self.presets.pending_step_state.clear();
        // An owed "after" state that lands from now on no longer describes
        // the live plugin; it only fills the snapshots that wait on it.
        for owed in self.presets.pending_after.values_mut() {
            owed.superseded = true;
        }
        for owed in self.plugin_mirror.owed_blobs.values_mut() {
            owed.superseded = true;
        }

        // Pause playback and stop recording. Recording should already be
        // blocked by `can_undo_redo_now`, but belt-and-braces.
        let _ = self.engine.send(AudioCommand::Stop);
        self.transport.playing = false;
        self.transport.recording = false;

        let UndoSnapshot {
            project: mut target,
            late_plugin_states,
        } = snapshot;
        for (instance_id, late) in late_plugin_states {
            let filled = late.lock().ok().and_then(|b| b.clone());
            if let Some(blob) = filled {
                target.plugin_states.insert(instance_id, blob);
            }
        }
        let project_path = self.io.project_path.clone();
        let ctx = ReconcileCtx {
            origin: Origin::Undo,
            project_dir: project_path.as_deref(),
            midi_notes: &target.midi_notes,
            plugin_states: &target.plugin_states,
        };
        // Every domain, in table order, by diff against `current`: the
        // transport / compose globals and the tempo map before any entity;
        // the routing edges, clips, plugin instances, tracks and busses the
        // target lacks, removed in that order; the entities, their plugin
        // state and order; the routing edges; the clips and what derives
        // from them; the app-side content; external instruments, lanes and
        // freeze last. See `docs/design/A-13-reconcile.md`.
        reconcile_all(self, Some(current), &target.file, &ctx);
    }

    /// Restore the compose section→clip map from `file` and raise the
    /// clip-id counter past everything restored — the one rule a disk load
    /// and an undo share (FU-H2a, ARCH-01 A-6, D-7b). Runs after the MIDI
    /// and audio clips are restored.
    ///
    /// The map is the file's (`ProjectFile::derived_clips`), not a
    /// rebuild from the mirror: a snapshot taken while a re-derived clip's
    /// `MidiClipCreated` echo was in flight holds that clip's entry but
    /// not the clip. An entry whose clip is not mirrored after the restore
    /// is kept only if that clip is in `echoes_in_flight` — the echoes
    /// still pending when the restore began (an undo's live map entries
    /// the live mirror lacked): the engine still has the clip and the
    /// echo will land, so dropping the entry would orphan it (FU-H2a).
    /// Every other unmirrored entry can never be satisfied and is
    /// dropped — after a disk load (`echoes_in_flight` empty) the engine
    /// holds only the replayed clips, and after an undo across an echo
    /// that has since landed the restore removed the clip (FU-A13j).
    /// Left in, it would suspend the UPD-05 freeze check on its track
    /// (see `revalidate_frozen_content`). A file saved before the field
    /// existed gets the positional rebuild.
    ///
    /// The counter (`EntityIds::clips`) is not snapshot state and is never
    /// lowered — not by an undo, not by a disk load (D-7b) — so an id the
    /// redo stack, a backup or another open project still names (a vocal
    /// render's `clip_<id>.wav` among them) is never re-issued (STATE-08).
    /// It is only raised here: past every restored MIDI clip, audio clip
    /// and map value (the last covering a pending clip whose echo lands
    /// after the restore), and past every audio take's `clip_ref`, which
    /// names a `clip_<id>.wav` no mirrored clip carries. A loaded project
    /// may come from a session whose counter ran further than this one's.
    pub(crate) fn restore_derived_clips(
        &mut self,
        file: &crate::project::ProjectFile,
        echoes_in_flight: &std::collections::HashSet<ClipId>,
    ) {
        match &file.derived_clips {
            Some(entries) => {
                let midi_clips = &self.midi_clips;
                self.compose.derived_clips = entries
                    .iter()
                    .filter(|e| {
                        echoes_in_flight.contains(&e.clip_id)
                            || midi_clips.iter().any(|mc| mc.id == e.clip_id)
                    })
                    .map(|e| ((e.definition_id, e.placement_id, e.track_id), e.clip_id))
                    .collect();
            }
            None => {
                let drum_track_ids =
                    crate::compose::ComposeState::drum_track_ids(&self.registry.tracks);
                self.compose
                    .rebuild_derived_clips(&self.midi_clips, &self.tempo_map, &drum_track_ids);
            }
        }
        let take_refs = file.take_groups.iter().flat_map(|g| &g.takes).filter_map(|t| {
            match t.content {
                resonance_common::TakeContent::Audio { clip_ref } => Some(clip_ref),
                resonance_common::TakeContent::Midi { .. } => None,
            }
        });
        // The file's own clip lists as well as the mirror: a clip the
        // restore skipped (its WAV unreadable) still names its id.
        self.media.ids.clips.seed_past(
            self.midi_clips
                .iter()
                .map(|mc| mc.id)
                .chain(self.clips.iter().map(|c| c.id))
                .chain(file.midi_clips.iter().map(|mc| mc.id))
                .chain(file.clips.iter().map(|c| c.id))
                .chain(self.compose.derived_clips.values().copied())
                .chain(take_refs),
        );
    }

    /// Drive the engine + GUI external-instrument state to `target`'s
    /// `ProjectTrack::external_instrument`s — the one restore for a disk
    /// load and both undo/redo paths (the `ExternalInstruments` reconcile
    /// domain, ARCH-01 A-13b).
    ///
    /// Clears tracks that are no longer external, then (re-)asserts every
    /// target config via `SetExternalInstrument` in file order — idempotent
    /// on the engine and, unlike a patch send, it never re-fires MIDI to the
    /// synth (a disk load re-sends the patches afterwards, in the
    /// `AllCleared` handler). The selected device preset (epic #40) is
    /// restored too: the id is re-applied to the GUI state and the resolved
    /// params re-sent via `SetTrackDeviceParams`, so device selection
    /// reverses with the rest of the config. The runtime device-offline
    /// flags are preserved for tracks that stay external (live hardware
    /// status survives an undo); a track returning to external mode starts
    /// online and is re-checked on the next ping.
    ///
    /// `after_clear_all`: the engine was just emptied by `ClearAll`, so
    /// nothing is cleared on it — the app map is simply dropped (every
    /// track starts online) — and a track with no device selected gets no
    /// `SetTrackDeviceParams` (the engine has no bindings to clear; a
    /// project from before device presets loads with no device traffic).
    /// Without a clear, an empty map is sent so a deselected device leaves
    /// no stale bindings behind.
    ///
    /// `fresh`: tracks the restore has just added to the engine (ARCH-01
    /// A-13i — on an undo, those `old` did not have or had with a
    /// different shape). Each gets the after-`ClearAll` treatment on its
    /// own: it starts online, and no empty `SetTrackDeviceParams` is sent
    /// for it when no device is selected.
    pub(crate) fn restore_external_instruments(
        &mut self,
        target: &crate::project::ProjectFile,
        after_clear_all: bool,
        fresh: &std::collections::HashSet<TrackId>,
    ) {
        if after_clear_all {
            self.devices.external_instruments.clear();
        }
        // Drop external mode from tracks absent in the target snapshot. Clear
        // their engine device-param map too, so a track leaving external mode
        // doesn't leave stale bindings behind on the engine side.
        let stale: Vec<TrackId> = self
            .devices
            .external_instruments
            .keys()
            .copied()
            .filter(|id| {
                !target
                    .tracks
                    .iter()
                    .any(|pt| pt.id == *id && pt.external_instrument.is_some())
            })
            .collect();
        for id in stale {
            self.devices.external_instruments.remove(&id);
            let _ = self
                .engine
                .send(AudioCommand::ClearExternalInstrument { track_id: id });
            let _ = self.engine.send(AudioCommand::SetTrackDeviceParams {
                track_id: id,
                params: Vec::new(),
            });
        }
        // Re-assert every target config, keeping live offline flags.
        for pt in &target.tracks {
            let Some(ext) = &pt.external_instrument else {
                continue;
            };
            let id = pt.id;
            let config = ext.config(id);
            let _ = self
                .engine
                .send(AudioCommand::SetExternalInstrument { config });
            // Restore the selected device preset and re-send its params
            // (an empty map when none is selected, unless there is nothing
            // on the engine to clear). An unresolved id also sends an empty
            // map, and the selection is kept so a later rescan can recover
            // it.
            let starts_clean = after_clear_all || fresh.contains(&id);
            if starts_clean {
                self.devices.external_instruments.remove(&id);
            }
            if !starts_clean || ext.device_id.is_some() {
                let params = ext.device_params(&self.devices.registry);
                let _ = self.engine.send(AudioCommand::SetTrackDeviceParams {
                    track_id: id,
                    params,
                });
            }
            let state = self
                .devices
                .external_instruments
                .entry(id)
                .or_insert_with(|| crate::state::ExternalInstrumentState::new(id));
            state.apply_config(&config);
            state.device_id = ext.device_id.clone();
        }
    }

    /// Reconcile the engine's automation lanes to `lanes` — the file form,
    /// `ProjectFile::automation_lanes` — and overwrite the app-side mirror
    /// to match. The one restore for a disk load and both undo/redo paths
    /// (the structure-preserving diff replay and the full
    /// clear-and-replay), and correct for each because the engine's
    /// lane set always equals the current mirror at call time: the diff
    /// path never touches lanes, and `ClearAll` deliberately leaves the
    /// engine's automation lanes intact (they are re-keyed per target,
    /// not by track/clip identity).
    ///
    /// The file form is lossless: the serializer writes the mirror's
    /// lanes verbatim (sorted by lane id), and the mirror is keyed by each
    /// lane's own target, so rebuilding the map here gives back the
    /// mirror that was saved.
    ///
    /// Lanes present now but absent from `lanes` are cleared; lanes that
    /// are new or whose breakpoints/Read flag changed are re-sent
    /// whole-lane. Transient live-value tints for dropped targets are
    /// discarded so a stale fader/knob tint can't outlive its lane.
    pub(crate) fn restore_automation_lanes(&mut self, lanes: &[AutomationLane]) {
        let target: HashMap<AutomationTarget, AutomationLane> = lanes
            .iter()
            .map(|lane| (lane.target.clone(), lane.clone()))
            .collect();
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
        for (t, lane) in &target {
            if self.automation.lanes.get(t) != Some(lane) {
                let _ = self
                    .engine
                    .send(AudioCommand::SetAutomationLane { lane: lane.clone() });
            }
        }
        self.automation
            .live_values
            .retain(|t, _| target.contains_key(t));
        self.automation.lanes = target;
        self.automation.bump_allocator_past_lanes();
    }

    /// Attempt to undo. No-ops (returning false) if the history is empty
    /// or an in-flight operation blocks undo/redo. On success the current
    /// state is pushed onto the redo stack and the popped snapshot is
    /// restored (the restore runs first — see `restore_from_snapshot_against`
    /// — but nothing observes the difference: the push only touches the
    /// undo history's own bookkeeping).
    pub(crate) fn try_undo(&mut self) -> Option<String> {
        if !self.can_undo_redo_now() || !self.session.undo.can_undo() {
            return None;
        }
        let (snapshot, label) = self.session.undo.pop_undo()?;
        // An open inline rename is dropped: the name under its field may be
        // about to change (or its track to go), and committing the buffer
        // afterwards would undo the undo.
        crate::update::inline_rename::cancel(self);
        // An import whose entry this undo pops must not place its clip
        // when the file lands later (code review UPD-04).
        self.media.pool_import.drop_undone(self.session.undo.undo_len());
        // Built once (FU-A13k): `restore_from_snapshot_against` diffs
        // against this same file instead of rebuilding it, since nothing
        // between here and there mutates the project (only the undo
        // history's own bookkeeping, pushed right after).
        let current = self.snapshot_for_undo();
        self.restore_from_snapshot_against(&current.project.file, snapshot);
        // The action just undone is what a redo would re-apply, so its
        // label travels with the state pushed onto the redo stack.
        self.session.undo.push_redo(current, label.clone());
        // An undo changes the song like any committed edit — remote
        // control clients detect it through the revision counter
        // (doc #265, todo #1147).
        self.bump_revision();
        Some(label)
    }

    /// Symmetric counterpart to `try_undo`.
    pub(crate) fn try_redo(&mut self) -> Option<String> {
        if !self.can_undo_redo_now() || !self.session.undo.can_redo() {
            return None;
        }
        let (snapshot, label) = self.session.undo.pop_redo()?;
        crate::update::inline_rename::cancel(self);
        // Built once (FU-A13k) — see `try_undo`.
        let current = self.snapshot_for_undo();
        self.restore_from_snapshot_against(&current.project.file, snapshot);
        self.session.undo.push_undo(current, label.clone());
        // Symmetric to `try_undo`: a redo is a committed edit for remote
        // revision-tracking purposes.
        self.bump_revision();
        Some(label)
    }
}
