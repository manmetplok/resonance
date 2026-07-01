//! Update handlers for audio media-pool import + placement (doc #175,
//! ba todo #598).
//!
//! This is the orchestration layer beneath the import entry points (the
//! "Import audio…" dialog and drag-and-drop, todo #608) and the pool
//! browser. It turns a multi-file selection into one
//! `AudioCommand::ImportAudioToPool` and, for a drop, records — per source
//! file — where the resulting asset should be placed. The placement itself
//! happens later, when each file's `AssetImported` event lands (see
//! `engine_events::pool`), because the engine assigns asset ids
//! asynchronously off-thread.
//!
//! **Undo.** Both messages are classified `UndoAction::Record`
//! (`undo::classify`), so `update()` captures a pre-import project snapshot
//! *before* this handler runs. That single snapshot is the whole undoable
//! action: one undo reverts the imported pool asset(s), any placed clip(s),
//! and a track spawned for a new-track drop — all of which ride the
//! `ProjectFile` snapshot/replay path. Nothing here records a *second*
//! undo entry when the asset later lands.

use iced::Task;
use resonance_audio::types::{AudioCommand, SamplePos};

use crate::message::{DropTarget, Message, PoolMessage};
use crate::state::{PendingImport, PlacementTarget};
use crate::Resonance;

pub fn handle(r: &mut Resonance, message: PoolMessage) -> Task<Message> {
    match message {
        PoolMessage::ImportFilesToPool(paths) => {
            import(r, paths, PlacementTarget::PoolOnly);
        }
        PoolMessage::ImportAndPlace { paths, target } => {
            let placement = resolve_target(r, target);
            import(r, paths, placement);
        }
    }
    Task::none()
}

/// Resolve a drop `target` into a concrete [`PlacementTarget`]: snap the
/// drop position to the grid and, for the new-track zone, reserve the
/// lane's id and issue its `AddTrack` up front so the clip can be placed
/// on it the moment the asset lands. The engine echoes `TrackAdded`
/// (mirrored by `engine_events::tracks::added` into a fresh audio
/// `TrackState`) well before the slower decode/transcode emits
/// `AssetImported`, so the target track always exists by placement time.
fn resolve_target(r: &mut Resonance, target: DropTarget) -> PlacementTarget {
    match target {
        DropTarget::ExistingTrack {
            track_id,
            start_sample,
        } => PlacementTarget::Track {
            track_id,
            start_sample: snap_drop_sample(r, start_sample),
        },
        DropTarget::NewTrack { start_sample } => {
            let track_id = r.registry.allocate_sub_track_id();
            let name = new_audio_track_name(r);
            let _ = r.engine.send(AudioCommand::AddTrack {
                id_hint: Some(track_id),
                name: Some(name),
            });
            PlacementTarget::Track {
                track_id,
                start_sample: snap_drop_sample(r, start_sample),
            }
        }
    }
}

/// Kick off an import batch: queue each source file's placement, then send
/// one `ImportAudioToPool` for the whole selection. A no-op on an empty
/// selection. Importing requires a project directory (the engine copies /
/// transcodes each file into `{project}/audio/`); without one the import
/// is refused with a user-facing error rather than silently dropped.
fn import(r: &mut Resonance, paths: Vec<std::path::PathBuf>, placement: PlacementTarget) {
    if paths.is_empty() {
        return;
    }
    if r.io.project_path.is_none() {
        r.error_message =
            Some("Save the project before importing audio, so imported files have a home.".into());
        return;
    }

    let path_strings: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    // Queue each file's placement. A place-drop of several files onto one
    // lane queues the same (snapped) position for each — they land stacked
    // at the drop point as independent, individually editable clips the
    // user can then drag apart. The common case (a single-file drop, or the
    // pool-only dialog import) needs no such spreading.
    for source_path in &path_strings {
        r.pool_import.push(PendingImport {
            source_path: source_path.clone(),
            target: placement,
        });
    }

    let _ = r.engine.send(AudioCommand::ImportAudioToPool {
        paths: path_strings,
    });
}

/// Snap a raw drop sample position to the timeline grid, reusing the exact
/// helper the clip-drag reducer uses so a dropped clip lands on the same
/// boundaries a dragged one would.
fn snap_drop_sample(r: &Resonance, raw: SamplePos) -> SamplePos {
    crate::view::timeline::snap_sample_to_grid(
        raw,
        r.transport.bpm,
        r.transport.time_sig_num,
        r.sample_rate,
        r.viewport.zoom,
    )
}

/// A unique default name for a track spawned by a new-track drop, e.g.
/// `"Audio 3"`. Counts existing tracks so repeated drops don't collide on
/// one label.
fn new_audio_track_name(r: &Resonance) -> String {
    format!("Audio {}", r.registry.tracks.len() + 1)
}
