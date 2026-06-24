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
//!   `restore_drum_patterns`, `restore_tempo_events`) used both here and
//!   by the diff-based undo replay path.

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
    restore_drum_patterns, restore_performance, restore_pool, restore_quantize,
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
    r.io.project_path = None; // Will be set by the caller (OpenPathSelected)

    // Wipe runtime-only vocal side-tables (clip_lyrics, render_epoch)
    // before re-installing entries from the project. Without this,
    // loading a project on top of an existing one keeps stale lyrics
    // for clips that no longer exist.
    r.compose.vocal_audio.clear();

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

    // Rehydrate parameter-automation lanes (architecture doc #162 §3).
    // Mirror them into app state and push each to the engine via
    // `SetAutomationLane` so the authoritative engine lanes round-trip
    // (the lane carries its own `enabled` Read flag). Done last so the
    // tracks/busses/plugins the lanes target already exist. Empty on
    // legacy projects ⇒ no commands, static values stay in effect.
    r.automation.load_lanes(project.automation_lanes.iter().cloned());
    for lane in r.automation.lanes.values().cloned().collect::<Vec<_>>() {
        let _ = r.engine.send(AudioCommand::SetAutomationLane { lane });
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

    // Reset transient UI state so the new project starts clean.
    r.viewport.scroll_offset = 0.0;
    r.viewport.scroll_offset_y = 0.0;
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
    // Drop external-instrument mode so the load starts clean. A fresh
    // project load re-asserts it per-track from `ProjectTrack.external_instrument`
    // in `replay_track`; an undo restore re-asserts it from `UndoExtras`
    // right after this runs (and its snapshot project file carries the same
    // config, so the two agree idempotently).
    r.external_instruments.clear();

    // Bump the app-side sub-track id counter past any persisted ids so
    // new sub-tracks allocated after this load don't collide with
    // restored ones. Saved projects from buggier prior versions may have
    // *non-sub-track* ids that fell into the sub-track range; include
    // every id so the next `allocate_sub_track_id` skip loop has fewer
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

/// Replay tracks, busses, master FX chain, and resolve track→bus routing.
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

        // Re-install the lyric side-table. Serializer strips trailing
        // empties; pad back to the clip's note count so every parallel
        // walker stays correctly aligned. Skip entirely when the saved
        // vec is all-empty (legacy projects + non-vocal clips).
        if !pmc.vocal_lyrics.is_empty() {
            let mut lyrics = pmc.vocal_lyrics.clone();
            lyrics.resize(note_count, String::new());
            r.compose.vocal_audio.clip_lyrics.insert(pmc.id, lyrics);
        }
    }
}

/// Rebuild vocal-derived state that depends on the already-restored MIDI and
/// audio clips: the `derived_clips` section→clip map (used by the compose
/// view) and the vocal-audio clip map (so the next Generate Vocal correctly
/// tears down old clips rather than stacking on top of them).
fn replay_vocal(r: &mut Resonance, project: &ProjectFile, loaded: &LoadedProject) {
    let samples_per_beat = r.sample_rate as f64 * 60.0 / r.transport.bpm as f64;
    let samples_per_bar = (samples_per_beat * r.transport.time_sig_num as f64) as u64;
    r.compose
        .rebuild_derived_clips(&r.midi_clips, samples_per_bar);

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
        samples_per_bar,
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
