//! Reconstruct GUI state from a `LoadedProject` and replay every required
//! engine command. Called after `AudioEvent::AllCleared` confirms the
//! engine has been emptied. Side-effecting end to end — sends ~20
//! `AudioCommand` variants and mutates almost every sub-state of `Resonance`.
//!
//! ## Module layout
//! - `mod.rs` (this file): public entry point [`replay_loaded_project`] and
//!   the seven per-domain helpers that structure it.
//! - `entity.rs`: per-entity replay (`replay_track`, `replay_bus`,
//!   `replay_master`, `replay_plugins`) plus `sort_plugins_by_saved_order`
//!   and `migrate_auto_name`.
//! - `restore.rs`: standalone restore helpers (`restore_performance`,
//!   `restore_quantize`, `restore_pool`, `restore_references`,
//!   `restore_drum_patterns`, `restore_tempo_events`, `replay_take_groups`)
//!   used both here and by the diff-based undo replay path.

mod entity;
mod restore;

use resonance_audio::types::*;

use crate::project::{LoadedProject, ProjectFile};
use crate::state::*;
use crate::util::db_to_gain;
use crate::Resonance;

// Re-export helpers consumed by sibling modules (undo replay, diff replay).
pub use entity::{migrate_auto_name, sort_plugins_by_saved_order};
pub(crate) use restore::{
    replay_take_groups, restore_drum_patterns, restore_performance, restore_pool, restore_quantize,
    restore_track_groups,
    restore_references, restore_tempo_events,
};

/// Snapshot of the saved plugin-chain ordering for every track, bus, and the
/// master chain. Collected from the project file *before* the registry is
/// wiped, then re-applied *after* all tracks and plugins have been replayed.
///
/// `track_added` / `bus_added` / `master_added` push a new slot at the end
/// whenever they see an `instance_id` that isn't already in the slot list —
/// a `PluginAdded` event arriving out of order would silently scramble the
/// saved chain. The post-replay sort in [`finalize_plugin_chains`] restores it.
struct SavedPluginOrder {
    tracks: std::collections::HashMap<TrackId, Vec<u64>>,
    busses: std::collections::HashMap<BusId, Vec<u64>>,
    master: Vec<u64>,
}

/// Replay a loaded project into the engine and rebuild GUI state. Called
/// after `AudioEvent::AllCleared` confirms the engine is empty.
pub fn replay_loaded_project(r: &mut Resonance, loaded: Box<LoadedProject>) {
    let project = &loaded.file;
    // Will be set by the caller (OpenPathSelected); an undo/redo's caller
    // puts this one back, which the freeze restore needs meanwhile.
    let live_project_path = r.io.project_path.take();

    // Wipe runtime-only vocal side-tables (clip_lyrics, render_epoch)
    // before re-installing entries from the project. Without this,
    // loading a project on top of an existing one keeps stale lyrics
    // for clips that no longer exist.
    r.compose.vocal_audio.clear();

    // Freeze status is restored from `ProjectTrack.freeze` at the end of
    // the replay (`replay_freeze`). A disk load drops the previously open
    // project's statuses, batch and UPD-05 baselines now. An undo/redo
    // keeps the live statuses until then, so `apply_freeze_restore` can
    // see which caches the restore retires, and keeps the baselines: a
    // track it brings back `Frozen` must still go stale on its next
    // content edit, and the baseline it was frozen with is not in the
    // snapshot (FU-H2b). The fingerprint is keyed by track id and slot
    // position, both stable across the replay. Nothing in the replay
    // reads the statuses.
    if r.io.pending_undo_extras.is_some() {
        r.freeze.queue = None;
    } else {
        r.freeze.reset();
    }

    // Missing-plugin warning (ba doc #275 P5, todo #1309). A genuine
    // disk load starts with a clean slate: every slot is re-added
    // optimistically and the engine's refusals raise the warning again
    // for THIS project. An undo/redo replay re-adds the same plugins and
    // gets the same refusals, so it would re-raise the modal on every
    // history step — the same reason the missing-FILE modal is opened
    // only for disk loads (`engine_events::project_io::all_cleared`).
    if r.io.pending_undo_extras.is_none() {
        r.missing_plugins.reset();
    } else {
        r.missing_plugins.dismiss();
    }

    // Point the engine at the loaded project's directory so that
    // subsequent imports and recordings stream into it.
    let _ = r.engine
        .send(AudioCommand::SetProjectDir(loaded.project_dir.clone()));

    // Restore global transport, compose sections, and engine settings.
    replay_globals(r, project);

    // Wipe the runtime registry and collect the saved plugin-chain order so
    // we can re-impose it after all async PluginAdded events have settled.
    let saved_plugin_order = wipe_registry(r, project);

    // Replay tracks, busses, and master FX chain.
    replay_tracks_and_busses(r, project, &loaded);

    // Restore audio and MIDI clips.
    replay_audio_clips(r, project, &loaded);
    replay_midi_clips(r, project, &loaded);

    // Rebuild vocal-derived state from the restored clips.
    replay_vocal(r, project, &loaded);

    // Re-impose the saved plugin-slot order and refresh the side-index.
    finalize_plugin_chains(r, &saved_plugin_order);

    // Restore independent sub-states.
    restore_references(r, project);
    restore_pool(r, project, &loaded.project_dir);
    restore_quantize(r, project);
    restore_performance(r, project);
    restore_track_groups(r, project);
    // Cycle-record take lanes (epic #15). Seeds the mirror emptied by
    // `wipe_registry` and resolves each audio take's recorded WAV against
    // the project directory, so a take whose file travelled with the
    // bundle comes back and one that didn't is flagged rather than lost.
    replay_take_groups(r, project, &loaded.project_dir);

    // Parameter-automation lanes (epic #14 / epic #40). Reconcile the
    // engine + app mirror to exactly the saved set: `restore_automation_lanes`
    // clears any lane left over from a previously-open project (ClearAll does
    // not touch engine automation) and (re-)sends every saved lane. This runs
    // last, so `DeviceParam` lanes are (re-)applied *after* each external
    // track's `SetTrackDeviceParams` (dispatched in `replay_track`) — the
    // engine already knows the bindings by the time the lane arrives. Legacy
    // projects carry no lanes, so this reduces to clearing stale ones and is
    // otherwise a no-op. An undo's diff replay makes the same call.
    r.restore_automation_lanes(&project.automation_lanes);

    // Last: a disk load's freeze baseline fingerprints the replayed
    // content, automation lanes included.
    replay_freeze(r, project, &loaded.project_dir, live_project_path.as_deref());
}

/// Restore every track's freeze status from `ProjectTrack.freeze` (ARCH-01
/// A-4). A disk load re-attaches each frozen track's cache to the engine
/// (ba todo #577); an undo/redo reconciles against the live statuses the
/// way the diff replay does, deleting the cache of a freeze it undoes —
/// against the live project path, which the undo caller restores after the
/// replay (`engine_events::project_io::all_cleared`).
fn replay_freeze(
    r: &mut Resonance,
    project: &ProjectFile,
    project_dir: &std::path::Path,
    live_project_path: Option<&std::path::Path>,
) {
    if r.io.pending_undo_extras.is_some() {
        r.apply_freeze_restore(&project.tracks, live_project_path);
    } else {
        let freezes: Vec<_> = project
            .tracks
            .iter()
            .filter(|t| t.freeze.is_frozen)
            .map(|t| (t.id, t.freeze.clone()))
            .collect();
        r.rehydrate_frozen_tracks(project_dir, &freezes);
    }
}

// ---------------------------------------------------------------------------
// Per-domain helpers
// ---------------------------------------------------------------------------

/// Restore global transport state (BPM, time signature, metronome, master
/// volume, MIDI clock, loop range), compose/section state (definitions,
/// placements, markers, drum patterns), and the tempo-event map. Sends all
/// corresponding engine commands so the engine is in sync before tracks and
/// clips are replayed.
fn replay_globals(r: &mut Resonance, project: &ProjectFile) {
    // Transport scalars.
    r.transport.bpm = project.bpm;
    r.transport.time_sig_num = project.time_sig_num;
    r.transport.time_sig_den = project.time_sig_den;
    r.transport.metronome_enabled = project.metronome_enabled;
    r.master_volume = project.master_volume;
    r.transport.loop_enabled = project.loop_enabled;
    r.transport.loop_in = project.loop_in;
    r.transport.loop_out = project.loop_out;
    r.transport.playhead = 0;

    // Reset transient UI state so the new project starts clean. The
    // scroll position is NOT reset here: this replay also serves a
    // slow-path undo, which must not move the view, and the horizontal
    // offset belongs to the outer `Scrollable` — a disk load scrolls that
    // for real in `engine_events::project_io::all_cleared` (code review
    // FU-V3a).
    r.interaction.selected_clip = None;
    r.mixer.selected_plugin = None;
    r.interaction.clip_drag = None;
    r.interaction.clip_trim = None;
    r.confirm_delete_track = None;
    r.confirm_quit = None;

    // Sections / compose.
    r.compose
        .load_from_project(&project.section_definitions, &project.section_placements);
    r.markers = crate::state::ArrangementMarkers::from(project.arrangement_markers.clone());
    // Global chord track (epic #33): app-side metadata only, nothing to
    // send. Legacy projects carry none and come up with an empty track.
    r.chord_track = project.chord_track.to_chord_track();

    // Restore the project's drum pattern bank (with legacy promotion),
    // keeping the `ComposeState::default()` bank in place when the
    // project predates drum groups entirely. Afterwards point the
    // right-rail / modal focus at a pattern that actually exists.
    restore_drum_patterns(&mut r.compose, project, false);
    let first_group_id = r
        .compose
        .drum_patterns
        .first()
        .and_then(|p| p.groups.first().map(|g| g.id));
    r.compose.drumroll.selected_group_id = first_group_id;
    r.compose.drumroll.managing_group_id = first_group_id;
    r.compose.drumroll.managing_pattern_id = r.compose.default_drum_pattern_id;

    // Tempo / signature events — must precede the engine commands below.
    restore_tempo_events(r, project);

    // Engine commands: tempo, signature, metronome, master volume.
    let _ = r.engine.send(AudioCommand::SetBpm {
        bpm: r.transport.bpm,
    });
    r.rebuild_and_send_tempo();
    // Chords past a section's end are refused by every edit; hold a
    // file to the same once the meter is known (code review FU-V4b).
    let trimmed = crate::update::compose::trim_chords_to_sections(r);
    if !trimmed.is_empty() {
        tracing::warn!("trimmed chords past the end of section(s) {trimmed:?} on load");
    }
    let _ = r.engine.send(AudioCommand::SetTimeSignature {
        numerator: r.transport.time_sig_num,
        denominator: r.transport.time_sig_den,
    });
    let _ = r.engine.send(AudioCommand::SetMetronomeEnabled {
        enabled: r.transport.metronome_enabled,
    });
    let _ = r.engine.send(AudioCommand::SetMasterVolume {
        volume: db_to_gain(r.master_volume),
    });

    // Restore MIDI clock settings. The engine treats `enabled=false`
    // as a no-op port-wise, so it's safe to send for legacy projects.
    r.midi_clock_send_enabled = project.midi_clock_send_enabled;
    r.midi_clock_send_device = project.midi_clock_send_device.clone();
    r.midi_clock_recv_enabled = project.midi_clock_recv_enabled;
    r.midi_clock_recv_device = project.midi_clock_recv_device.clone();
    let _ = r.engine.send(AudioCommand::SetMidiClockOutput {
        device: r.midi_clock_send_device.clone(),
        enabled: r.midi_clock_send_enabled,
    });
    let _ = r.engine.send(AudioCommand::SetMidiClockInput {
        device: r.midi_clock_recv_device.clone(),
        enabled: r.midi_clock_recv_enabled,
    });

    // Loop range.
    let _ = r.engine.send(AudioCommand::SetLoopRange {
        enabled: r.transport.loop_enabled,
        loop_in: r.transport.loop_in,
        loop_out: r.transport.loop_out,
    });
}

/// Wipe the GUI registry (tracks, busses, clips, plugins) and collect the
/// saved plugin-chain order from the project file. Returns the saved order
/// so [`finalize_plugin_chains`] can re-impose it after replay.
fn wipe_registry(r: &mut Resonance, project: &ProjectFile) -> SavedPluginOrder {
    r.registry.tracks.clear();
    r.registry.busses.clear();
    r.master_plugins.clear();
    r.master_fx_bypassed = false;
    r.clips.clear();
    r.midi_clips.clear();
    r.registry.next_track_order = 0;
    r.registry.next_bus_order = 0;
    r.plugin_index.clear();
    // Drop the previous project's send graph. `ClearAll` empties the
    // engine's aux-send table without echoing an `AuxSendRemoved` per
    // send, so the mirror has to be emptied here or the loaded project
    // inherits routes into busses that no longer exist. `replay_sends`
    // re-seeds it from the project file afterwards.
    r.aux.sends.clear();
    r.aux.last_rejection = None;
    // Likewise the key-routing mirror (ba todo #1311): `ClearAll` empties
    // the engine's route table without echoing a `SidechainRouteChanged`
    // per route, so loading a project on top of another would otherwise
    // leave the new one keying plugins from the old one's tracks.
    // `replay_sidechain_routes` re-seeds it from the project file.
    r.sidechain.clear();
    // Drop external-instrument mode so the load starts clean. Both a fresh
    // project load and an undo's full replay re-assert it per-track from
    // `ProjectTrack.external_instrument` in `replay_track`.
    r.external_instruments.clear();
    // Likewise the cycle-record take lanes (epic #15, todo #412). The
    // mirror is built purely from `TakeCaptured` echoes, and `ClearAll`
    // empties the engine's take-group map without echoing a removal per
    // group — so without this, loading project B kept project A's take
    // lanes, complete with `clip_ref`s naming WAVs in a DIFFERENT
    // project's `audio/` directory (or none at all). `replay_take_groups`
    // re-seeds the mirror from the project file afterwards, and only
    // adds, so this clear is the one thing dropping the old project's.
    // (`ClearAll` genuinely does empty the engine's map as of ba todo
    // #1394 — it did not when this comment was first written, which is
    // how a loaded project inherited the previous one's comp table.)
    r.take_groups.clear();

    // Bump the app-side sub-track id counter past any persisted ids so
    // new sub-tracks allocated after this load don't collide with
    // restored ones. Saved projects from buggier prior versions may have
    // *non-sub-track* ids that fell into the sub-track range; include
    // every id so the next `allocate_track_id` skip loop has fewer
    // iterations to do.
    for pt in &project.tracks {
        if pt.id >= r.registry.next_sub_track_id {
            r.registry.next_sub_track_id = pt.id + 1;
        }
    }

    // Stash saved plugin-slot order per track / bus / master so we can
    // re-apply it after all replays + late `PluginAdded` events have
    // resolved. See [`finalize_plugin_chains`] for the sort that restores it.
    let mut tracks =
        std::collections::HashMap::with_capacity(project.tracks.len());
    let mut busses =
        std::collections::HashMap::with_capacity(project.busses.len());
    for pt in &project.tracks {
        tracks.insert(pt.id, pt.plugins.iter().map(|p| p.instance_id).collect());
    }
    for pb in &project.busses {
        busses.insert(pb.id, pb.plugins.iter().map(|p| p.instance_id).collect());
    }
    let master = project
        .master_plugins
        .iter()
        .map(|p| p.instance_id)
        .collect();

    SavedPluginOrder { tracks, busses, master }
}

/// Replay tracks, busses, master FX chain, and resolve routing — both the
/// track→bus main outputs and the aux-send graph.
/// Tracks must be replayed before busses (the engine tracks must exist when
/// routing is set), and busses must exist before `SetTrackOutput` is sent.
fn replay_tracks_and_busses(
    r: &mut Resonance,
    project: &ProjectFile,
    loaded: &LoadedProject,
) {
    for pt in &project.tracks {
        entity::replay_track(r, pt, loaded);
    }
    // Defensive: older project files weren't guaranteed to be saved in
    // .order sequence, and replay relies on the registry staying sorted
    // by .order for the view layer's invariant.
    r.registry.resort_tracks();
    r.compose.refresh_track_count(&r.registry.tracks);

    // Migrate old generate_params + track roles to lane_generators for
    // projects predating the unified lane generator system.
    r.compose.migrate_old_generate_params(&r.registry.tracks);

    // Replay busses (must come before SetTrackOutput so the target bus
    // exists at the time the routing is set).
    for pb in &project.busses {
        entity::replay_bus(r, pb, loaded);
    }
    r.registry.resort_busses();
    // Output-destination picker depends on the bus list.
    r.view_caches.rebuild_output(&r.registry.busses);

    // Replay master FX chain + bypass state.
    entity::replay_master(r, project, loaded);

    // Now that all busses exist, resolve track → bus routing.
    for pt in &project.tracks {
        if let Some(bus_id) = pt.output_bus {
            let _ = r.engine.send(AudioCommand::SetTrackOutput {
                track_id: pt.id,
                output: TrackOutput::Bus(bus_id),
            });
        }
    }

    // …and the aux-send graph, whose source tracks and destination busses
    // both have to exist first (the engine rejects a send naming either
    // one before it is registered).
    replay_sends(r, project);

    // …and the sidechain key routes, which need BOTH the source entity and
    // the target plugin to have been replayed — hence last, after the
    // track, bus and master chains have all gone out.
    replay_sidechain_routes(r, project);
}

/// Re-register the saved aux sends (ba doc #273) with the engine and seed
/// the GUI mirror.
///
/// Each send goes back as a `SetAuxSend` carrying its saved id as
/// `id_hint`, which the engine honours (and bumps its own allocator past),
/// so send ids survive a reload. The engine re-validates every route and
/// re-clamps every level, then echoes `AuxSendChanged` — which overwrites
/// the seeded entry with the engine-resolved one, keeping the engine the
/// authority on what is live. Seeding here rather than waiting for that
/// echo mirrors what every other entity in this module does (a track's
/// `TrackState` is pushed alongside its `AddTrack`), so the mixer is
/// correct the moment the load returns.
///
/// Legacy projects carry no sends, so this is a no-op for them — the
/// mirror was already emptied by [`wipe_registry`].
fn replay_sends(r: &mut Resonance, project: &ProjectFile) {
    for ps in &project.sends {
        // An unknown source kind means this build cannot tell what the
        // send is routed FROM. Dropping it loses a route; guessing wires
        // the wrong signal into a bus and is worse, so drop.
        let Some(source) = crate::project::send_source_from_tag(&ps.source_kind, ps.source_id)
        else {
            // Loud, because the next save rewrites the file without it:
            // a silent drop turns "this build does not understand one
            // edge" into permanent data loss with nothing to notice.
            tracing::warn!(
                "project load: dropping send {} — unknown source kind {:?}",
                ps.id, ps.source_kind
            );
            continue;
        };
        // Only mirror a send whose endpoints exist in the project we
        // just loaded. The engine rejects a send with a missing source
        // or destination, and `AuxSendRejected` does not remove a
        // seeded entry — so seeding one unconditionally leaves a phantom
        // the mixer draws and `song.tracks` reports while no audio is
        // routed, with no way back short of deleting it by hand.
        let source_exists = match source {
            SendSource::Track(id) => r.registry.tracks.iter().any(|t| t.id == id),
            SendSource::Bus(id) => r.registry.busses.iter().any(|b| b.id == id),
        };
        if !source_exists || !r.registry.busses.iter().any(|b| b.id == ps.dest_bus) {
            tracing::warn!(
                "project load: dropping send {} — endpoint missing (source {:?}, dest bus {})",
                ps.id, source, ps.dest_bus
            );
            continue;
        }
        let _ = r.engine.send(AudioCommand::SetAuxSend {
            id_hint: Some(ps.id),
            source,
            dest: ps.dest_bus,
            level_db: ps.level_db,
            pre_fader: ps.pre_fader,
            enabled: ps.enabled,
        });
        r.aux.upsert(AuxSend {
            id: ps.id,
            source,
            dest: ps.dest_bus,
            level_db: ps.level_db,
            pre_fader: ps.pre_fader,
            enabled: ps.enabled,
        });
    }
}

/// Every plugin instance id the project file carries, across track, bus
/// and master chains. The membership test for a saved key route's
/// target: a route onto a plugin this project no longer contains is
/// dropped rather than replayed.
///
/// Derived from the project file rather than from `r.plugin_index`
/// because the index is only rebuilt in `finalize_plugin_chains`, after
/// this runs — and because the file is the thing being validated.
fn saved_plugin_instance_ids(project: &ProjectFile) -> std::collections::HashSet<u64> {
    project
        .tracks
        .iter()
        .flat_map(|t| t.plugins.iter())
        .chain(project.busses.iter().flat_map(|b| b.plugins.iter()))
        .chain(project.master_plugins.iter())
        .map(|p| p.instance_id)
        .collect()
}

/// Re-register the saved sidechain key routes (ba doc #157/#159, todo
/// #1311) with the engine and seed the GUI mirror.
///
/// Each route goes back as a `SetSidechainRoute` naming the target
/// plugin's saved instance id, which is the same id `replay_plugins`
/// just handed the engine as an `id_hint` — so a route survives a reload
/// without any id remapping. The engine echoes `SidechainRouteChanged`,
/// which overwrites the seeded entry with the resolved one; seeding here
/// rather than waiting for that echo is what every other entity in this
/// module does, and it is what makes a save taken immediately after a
/// load write the same routes back out.
///
/// Legacy projects carry no routes, so this is a no-op for them — the
/// mirror was already emptied by [`wipe_registry`].
fn replay_sidechain_routes(r: &mut Resonance, project: &ProjectFile) {
    if project.sidechain_routes.is_empty() {
        return;
    }
    let known_plugins = saved_plugin_instance_ids(project);
    for pr in &project.sidechain_routes {
        // An unknown source kind means this build cannot tell what the
        // route is keyed FROM. Dropping it loses a duck; guessing points
        // the detector at a different channel — track and bus ids are
        // independent namespaces that both start at 1 — and a wrongly
        // keyed compressor is far harder to notice than an unkeyed one.
        let Some(source) = crate::project::send_source_from_tag(&pr.source_kind, pr.source_id)
        else {
            // Loud, because the next save rewrites the file without it.
            tracing::warn!(
                "project load: dropping sidechain route onto plugin {} — unknown source kind {:?}",
                pr.plugin_instance_id, pr.source_kind
            );
            continue;
        };
        let source_exists = match source {
            SendSource::Track(id) => r.registry.tracks.iter().any(|t| t.id == id),
            SendSource::Bus(id) => r.registry.busses.iter().any(|b| b.id == id),
        };
        if !source_exists || !known_plugins.contains(&pr.plugin_instance_id) {
            // Same reasoning as the send case: seeding a route whose
            // endpoints are missing leaves a phantom the mixer would draw
            // while no key is delivered at all.
            tracing::warn!(
                "project load: dropping sidechain route onto plugin {} — endpoint missing \
                 (source {:?})",
                pr.plugin_instance_id, source
            );
            continue;
        }
        let _ = r.engine.send(AudioCommand::SetSidechainRoute {
            plugin: pr.plugin_instance_id,
            source,
            enabled: pr.enabled,
        });
        r.sidechain.upsert(SidechainRoute {
            plugin: pr.plugin_instance_id,
            source,
            enabled: pr.enabled,
        });
    }
}

/// Replay audio clips from the project's clip list: hand the engine an
/// absolute path to each WAV file, push non-default fades/gain, and
/// build the corresponding `ClipState` entries.
fn replay_audio_clips(r: &mut Resonance, project: &ProjectFile, loaded: &LoadedProject) {
    for pc in &project.clips {
        let abs_path = loaded.project_dir.join(&pc.audio_file);
        let _ = r.engine.send(AudioCommand::LoadClipFromWav {
            clip_id: pc.id,
            track_id: pc.track_id,
            start_sample: pc.start_sample,
            path: abs_path,
            name: pc.name.clone(),
            trim_start_frames: pc.trim_start_frames,
            trim_end_frames: pc.trim_end_frames,
        });

        // Fades & per-clip gain (epic #18, doc #156). `LoadClipFromWav`
        // carries no fade/gain, so push them explicitly after the clip
        // exists; the engine clamps and echoes them back. Only emit when
        // non-default to keep legacy/unfaded projects quiet.
        let fade_in_frames = pc.fade_in_frames;
        let fade_in_curve = fade_curve_from_tag(&pc.fade_in_curve);
        let fade_out_frames = pc.fade_out_frames;
        let fade_out_curve = fade_curve_from_tag(&pc.fade_out_curve);
        let gain_db = pc.gain_db;
        if fade_in_frames != 0 || fade_out_frames != 0 {
            let _ = r.engine.send(AudioCommand::SetClipFade {
                clip_id: pc.id,
                fade_in_frames,
                fade_in_curve,
                fade_out_frames,
                fade_out_curve,
            });
        }
        if gain_db != 0.0 {
            let _ = r.engine.send(AudioCommand::SetClipGain {
                clip_id: pc.id,
                gain_db,
            });
        }

        let duration_samples = pc
            .total_frames
            .saturating_sub(pc.trim_start_frames)
            .saturating_sub(pc.trim_end_frames);
        r.clips.push(ClipState {
            id: pc.id,
            track_id: pc.track_id,
            start_sample: pc.start_sample,
            duration_samples,
            name: pc.name.clone(),
            total_frames: pc.total_frames,
            trim_start_frames: pc.trim_start_frames,
            trim_end_frames: pc.trim_end_frames,
            fade_in_frames,
            fade_in_curve,
            fade_out_frames,
            fade_out_curve,
            gain_db,
            waveform_peaks: Vec::new(), // Populated by ClipImported event.
            vocal_tuning: None,         // Re-derived on demand when the pitch editor opens.
            // Link to the pool asset this clip was placed from, if any
            // (doc #175). Persisted on `ProjectClip`; rebuilt here so
            // imported audio survives reload. `restore_pool` reconciles
            // it against the pool (and recomputes usage) after every clip
            // is in place.
            asset_ref: pc.asset_ref.map(crate::state::pool::AssetRef::new),
        });
    }
}

/// Replay MIDI clips from the parsed `.mid` files and restore the vocal
/// lyric side-table for each clip that carries lyrics.
fn replay_midi_clips(r: &mut Resonance, project: &ProjectFile, loaded: &LoadedProject) {
    for pmc in &project.midi_clips {
        let notes: Vec<MidiNote> = loaded.midi_notes.get(&pmc.id).cloned().unwrap_or_default();

        let _ = r.engine.send(AudioCommand::LoadMidiClipDirect {
            clip_id: pmc.id,
            track_id: pmc.track_id,
            start_sample: pmc.start_sample,
            duration_ticks: pmc.duration_ticks,
            notes: notes.clone(),
            name: pmc.name.clone(),
            trim_start_ticks: pmc.trim_start_ticks,
            trim_end_ticks: pmc.trim_end_ticks,
        });

        let note_count = notes.len();
        r.midi_clips.push(MidiClipState {
            id: pmc.id,
            track_id: pmc.track_id,
            start_sample: pmc.start_sample,
            duration_ticks: pmc.duration_ticks,
            name: pmc.name.clone(),
            notes,
            trim_start_ticks: pmc.trim_start_ticks,
            trim_end_ticks: pmc.trim_end_ticks,
        });

        // Re-install the lyric side-table in its live form: padded to
        // the clip's note count, no entry when the saved vec is empty
        // (legacy projects + non-vocal clips).
        r.compose
            .vocal_audio
            .restore_clip_lyrics(pmc.id, &pmc.vocal_lyrics, note_count);
    }
}

/// Rebuild vocal-derived state that depends on the already-restored MIDI and
/// audio clips: the `derived_clips` section→clip map (used by the compose
/// view) and the vocal-audio clip map (so the next Generate Vocal correctly
/// tears down old clips rather than stacking on top of them).
fn replay_vocal(r: &mut Resonance, project: &ProjectFile, loaded: &LoadedProject) {
    // Drum lanes carry no `lane_generators` entry, so the rebuild needs
    // the drum-track set to claim their clips too — without it the first
    // `generate.drums` after a load duplicates every drum clip instead of
    // replacing it (see `rebuild_derived_clips`).
    let drum_track_ids = crate::compose::ComposeState::drum_track_ids(&r.registry.tracks);
    r.compose
        .rebuild_derived_clips(&r.midi_clips, &r.tempo_map, &drum_track_ids);

    // Rebuild the vocal audio clip map so subsequent regen tear-downs
    // find the loaded clips and clean them up — otherwise the next
    // Generate Vocal stacks a new clip on top of the old one and the
    // mixer plays both summed together.
    use std::collections::{HashMap, HashSet};
    let vocal_track_ids: HashSet<resonance_audio::types::TrackId> = r
        .registry
        .tracks
        .iter()
        .filter(|t| t.track_type == resonance_audio::types::TrackType::Vocal)
        .map(|t| t.id)
        .collect();
    let audio_clip_paths: HashMap<resonance_audio::types::ClipId, std::path::PathBuf> = project
        .clips
        .iter()
        .map(|pc| (pc.id, loaded.project_dir.join(&pc.audio_file)))
        .collect();
    r.compose.rebuild_vocal_audio_clips(
        &r.clips,
        &audio_clip_paths,
        &vocal_track_ids,
        &r.tempo_map,
    );

    r.transport.loop_range_set = r.transport.loop_enabled;
}

/// Re-impose the saved plugin-chain order on every track, bus, and the
/// master chain, then rebuild the `plugin_index` side-index. Sorting in
/// place is cheap (Rust's sort is adaptive — already-sorted slices are O(n))
/// and safely no-ops in the common case where placeholders + events landed
/// in the expected order.
fn finalize_plugin_chains(r: &mut Resonance, saved: &SavedPluginOrder) {
    for track in &mut r.registry.tracks {
        if let Some(order) = saved.tracks.get(&track.id) {
            sort_plugins_by_saved_order(&mut track.plugins, order);
        }
    }
    for bus in &mut r.registry.busses {
        if let Some(order) = saved.busses.get(&bus.id) {
            sort_plugins_by_saved_order(&mut bus.plugins, order);
        }
    }
    sort_plugins_by_saved_order(&mut r.master_plugins, &saved.master);

    // Re-populate the `with_plugin_mut` side-index from the wholesale
    // replay we just performed. Per-slot inserts would also work but
    // a single rebuild is simpler and keeps the entity helpers focused
    // on their own concern.
    r.rebuild_plugin_index();
}

// Needed by replay_audio_clips; imported via the crate's project module.
use crate::project::fade_curve_from_tag;
