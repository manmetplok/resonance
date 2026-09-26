//! Reconstruct GUI state from a `LoadedProject` and replay every required
//! engine command. Called after `AudioEvent::AllCleared` confirms the
//! engine has been emptied. Side-effecting end to end — sends ~20
//! `AudioCommand` variants and mutates almost every sub-state of `Resonance`.
//!
//! ## Module layout
//! - `mod.rs` (this file): public entry point [`replay_loaded_project`] and
//!   the per-domain helpers that structure it.
//! - `entity.rs`: per-entity replay (`replay_track`, `replay_bus`,
//!   `replay_master`, `replay_plugins`) plus `sort_plugins_by_saved_order`
//!   and `migrate_auto_name`.
//! - `restore.rs`: standalone restore helpers (`restore_performance`,
//!   `restore_quantize`, `restore_pool`, `restore_references`,
//!   `restore_drum_patterns`, `restore_tempo_events`, `replay_take_groups`)
//!   used both here and by the diff-based undo replay path.
//!
//! Domains migrated to the `Reconcile` driver (`super::reconcile`, ARCH-01
//! A-13) are not restored inline here: [`replay_loaded_project`] opens
//! with `Stage::Globals` and `Stage::Timeline`, runs `Stage::Routing` and
//! `Stage::Clips` after the tracks, busses and master, and ends with
//! `Stage::Content` and `Stage::Tail` — the same stages, in the same
//! sequence, `try_diff_replay` runs.

mod entity;
mod restore;

use resonance_audio::types::*;

use super::reconcile::{reconcile_stage, LiveCarry, Origin, ReconcileCtx, Stage};
use crate::project::{LoadedProject, ProjectFile};
use crate::Resonance;

// Re-export helpers consumed by sibling modules (undo replay, diff replay).
pub use entity::{migrate_auto_name, sort_plugins_by_saved_order};
pub(crate) use restore::{
    replay_take_groups, restore_drum_patterns, restore_performance, restore_pool,
    restore_pool_assets, restore_quantize, restore_track_groups,
    reconcile_references, restore_references, restore_tempo_events, ReferenceMonitorSource,
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
    // `io.restoring_undo` marks an undo/redo's full replay (set at its
    // `ClearAll`, cleared by `all_cleared` after this returns); the
    // replay reads it only through the ctx.
    let origin = if r.io.restoring_undo {
        Origin::UndoFull
    } else {
        Origin::DiskLoad
    };
    r.io.reconcile_trace.clear();
    // Will be set by the caller (OpenPathSelected); an undo/redo's caller
    // puts this one back. The freeze restore needs it meanwhile, so the
    // ctx carries it.
    let live_project_path = r.io.project_path.take();
    let ctx = ReconcileCtx {
        origin,
        project_dir: Some(&loaded.project_dir),
        midi_notes: &loaded.midi_notes,
        live: LiveCarry {
            project_path: live_project_path.as_deref(),
            // An undo/redo never lowers the derived-clip id counter
            // (ARCH-01 A-6); `load_from_project` resets it, so remember it
            // here.
            derived_counter_floor: LiveCarry::derived_counter_floor(r, origin),
        },
    };

    // Wipe runtime-only vocal side-tables (clip_lyrics, render_epoch)
    // before re-installing entries from the project. Without this,
    // loading a project on top of an existing one keeps stale lyrics
    // for clips that no longer exist.
    r.compose.vocal_audio.clear();

    // Point the engine at the loaded project's directory so that
    // subsequent imports and recordings stream into it.
    let _ = r.engine
        .send(AudioCommand::SetProjectDir(loaded.project_dir.clone()));

    // Transport / master scalars (every one sent: `old` is `None`), the
    // transient UI reset, compose sections and the drum-pattern bank;
    // then tempo / signature events (+ `SetTempoEvents`), chord track,
    // markers and the section chord trim — all before any track or clip.
    reconcile_stage(r, Stage::Globals, None, project, &ctx);
    reconcile_stage(r, Stage::Timeline, None, project, &ctx);

    // Wipe the runtime registry and collect the saved plugin-chain order so
    // we can re-impose it after all async PluginAdded events have settled.
    let saved_plugin_order = wipe_registry(r, project);

    // Replay tracks, busses, master FX chain and the track outputs.
    replay_tracks_and_busses(r, project, &loaded);

    // The aux sends, then the sidechain key routes (every one sent: `old`
    // is `None`), once every endpoint they name — tracks, busses and the
    // master chain's plugin ids — has gone out.
    reconcile_stage(r, Stage::Routing, None, project, &ctx);

    // The audio and MIDI clips (every one loaded: `old` is `None`), then
    // the state derived from them: the lyric side-table (padded to the
    // replayed note counts), the derived-clip map (ARCH-01 A-6, keeping
    // only entries whose clip this replay installed — `ClearAll` wiped
    // anything else) and the vocal audio-clip map.
    reconcile_stage(r, Stage::Clips, None, project, &ctx);

    // Re-impose the saved plugin-slot order and refresh the side-index.
    finalize_plugin_chains(r, &saved_plugin_order);

    // References, pool (after the clips, whose asset refs it counts),
    // quantize, performance, track groups, and the cycle-record take lanes
    // (each audio take's WAV resolved against the project directory, so a
    // take whose file travelled with the bundle comes back and one that
    // didn't is flagged rather than lost).
    reconcile_stage(r, Stage::Content, None, project, &ctx);

    // External instruments, then the automation lanes (a `DeviceParam`
    // lane needs the device bindings the first sends), the missing-plugin
    // warning, and freeze last (a disk load's baseline fingerprints the
    // replayed content, lanes included).
    reconcile_stage(r, Stage::Tail, None, project, &ctx);
}

// ---------------------------------------------------------------------------
// Per-domain helpers
// ---------------------------------------------------------------------------

/// Wipe the GUI registry (tracks, busses, clips, plugins) and collect the
/// saved plugin-chain order from the project file. Returns the saved order
/// so [`finalize_plugin_chains`] can re-impose it after replay.
fn wipe_registry(r: &mut Resonance, project: &ProjectFile) -> SavedPluginOrder {
    r.registry.tracks.clear();
    r.registry.busses.clear();
    r.master_plugins.clear();
    r.master_fx_bypassed = false;
    // The audio and MIDI clip mirrors are emptied by their own reconcile
    // domains (`AudioClips`, `MidiClips`), just before they reload them;
    // nothing in between reads them.
    r.registry.next_track_order = 0;
    r.registry.next_bus_order = 0;
    r.plugin_mirror.index.clear();
    // The aux-send and key-route mirrors are emptied by their own reconcile
    // domains (`Sends`, `SidechainRoutes`, `Stage::Routing`), just before
    // they re-seed them; nothing in between reads them.
    // External-instrument mode is dropped and re-asserted by its own
    // reconcile domain (`ExternalInstruments`, `Stage::Tail`); nothing in
    // between reads it.
    // The cycle-record take lanes are cleared by their own reconcile
    // domain (`reconcile::app_side::TakeGroups`), just before it re-seeds
    // them; nothing in between reads them.

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

/// Replay tracks, busses, master FX chain, and resolve the track→bus main
/// outputs. The aux sends and key routes follow in `Stage::Routing`.
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

/// Re-impose the saved plugin-chain order on every track, bus, and the
/// master chain, then rebuild the `plugin_mirror.index` side-index. Sorting in
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
