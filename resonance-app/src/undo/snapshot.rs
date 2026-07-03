//! Undo snapshot data model and the `Resonance` methods that build, restore,
//! and apply snapshots.
//!
//! Three kinds of types live here:
//! - Capture types that shadow fields not yet covered by `ProjectFile`
//!   (`ClipFadeGain`, `UndoExtras`, `UndoSnapshot`).
//! - The `CoalesceKey` discriminator used by the history stack to merge a
//!   burst of fader/knob messages into one undo entry.
//! - The `impl crate::Resonance` blocks for building a snapshot
//!   (`snapshot_for_undo`), checking preconditions, driving undo/redo, and
//!   applying a restored snapshot back to the live engine.

use std::collections::HashMap;

use resonance_audio::types::{AudioCommand, ClipId, FadeCurve, MidiNote, PluginInstanceId};
use resonance_common::ExternalInstrument;

use crate::project::LoadedProject;
use resonance_audio::types::TrackId;

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
