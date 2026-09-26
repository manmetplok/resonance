//! Reconstruct GUI state from a `LoadedProject` and replay every required
//! engine command. Called after `AudioEvent::AllCleared` confirms the
//! engine has been emptied. Side-effecting end to end — sends ~20
//! `AudioCommand` variants and mutates almost every sub-state of `Resonance`.
//!
//! ## Module layout
//! - `mod.rs` (this file): public entry point [`replay_loaded_project`] and
//!   the re-exported restore helpers.
//! - `restore.rs`: standalone restore helpers (`restore_performance`,
//!   `restore_quantize`, `restore_pool`, `restore_references`,
//!   `restore_drum_patterns`, `restore_tempo_events`, `replay_take_groups`)
//!   used both here and by the diff-based undo replay path.
//!
//! Domains migrated to the `Reconcile` driver (`super::reconcile`, ARCH-01
//! A-13) are not restored inline here: all [`replay_loaded_project`] does
//! is `SetProjectDir` and every `Stage` in sequence (`reconcile_all_stages`)
//! — the same stages, in the same sequence, `try_diff_replay` runs.

mod restore;

use resonance_audio::types::*;

use super::reconcile::{reconcile_stage, LiveCarry, Origin, ReconcileCtx, Stage};
use crate::project::LoadedProject;
use crate::Resonance;

// Re-export helpers consumed by sibling modules (undo replay, diff replay).
// The registry wipe, the entity replay and the plugin-chain finalisation
// that used to live here are the `Stage::Entities` reconcile domains
// (ARCH-01 A-13f).
pub use super::reconcile::{migrate_auto_name, sort_plugins_by_saved_order};
pub(crate) use restore::{
    replay_take_groups, restore_drum_patterns, restore_performance, restore_pool,
    restore_pool_assets, restore_quantize, restore_track_groups,
    reconcile_references, restore_references, restore_tempo_events, ReferenceMonitorSource,
};

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
        plugin_states: &loaded.plugin_states,
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

    // Tracks (with their plugin chains), busses, the master chain, the
    // track outputs once every bus exists, each plugin's blob / bypass /
    // parked params, then the registry resort, the saved plugin-slot order
    // and the plugin side-index — each registry emptied by its own domain
    // first (every entity added: `old` is `None`).
    reconcile_stage(r, Stage::Entities, None, project, &ctx);

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
