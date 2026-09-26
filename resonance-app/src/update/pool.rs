//! Update handlers for audio media-pool import + placement (doc #175,
//! ba todo #598 / #608).
//!
//! This is the orchestration layer beneath the import entry points (the
//! "Import audio…" chrome button and the window file-drop subscription,
//! todo #608) and the pool browser drag-to-timeline gesture (todo #605).
//! It turns a multi-file selection into one `AudioCommand::ImportAudioToPool`
//! and, for a drop, records — per source file — where the resulting asset
//! should be placed. The placement itself happens later, when each file's
//! `AssetImported` event lands (see `engine_events::pool`), because the
//! engine assigns asset ids asynchronously off-thread.
//!
//! **Undo.** `ImportFilesToPool` and `ImportAndPlace` are classified
//! `UndoAction::Record` (`undo::classify`), so `update()` captures a
//! pre-import project snapshot *before* this handler runs. That single
//! snapshot is the whole undoable action: one undo reverts the imported
//! pool asset(s), any placed clip(s), and a track spawned for a new-track
//! drop — all of which ride the `ProjectFile` snapshot/replay path. Nothing
//! here records a *second* undo entry when the asset later lands. The two
//! entry-point helpers (`PickFiles` / `WindowAudioDrop`) are classified
//! `UndoAction::Skip` — they carry no state of their own.

use std::path::Path;

use iced::Task;
use resonance_audio::types::{AssetId, AudioCommand, ClipId, SamplePos, TrackId};

use crate::message::{DropTarget, Message};
use crate::state::{PendingImport, PlacementTarget};
use crate::Resonance;

/// Audio import + placement orchestration (doc #175, ba todo #598).
/// Drives the end-to-end flow: a multi-file selection (from the "Import
/// audio…" dialog or a drag-and-drop) is imported into the project pool
/// via `AudioCommand::ImportAudioToPool`, and — for a drop — each file is
/// placed as an audio clip once its `AssetImported` event lands. Routed
/// through `update::pool::handle`.
///
/// `ImportFilesToPool` and `ImportAndPlace` are classified
/// `UndoAction::Record` (see `undo::classify`) so the whole import +
/// placement is a single undoable action: the undo snapshot is taken up
/// front, before the import is issued, so one undo reverts the pool
/// asset(s), any placed clip(s), and a spawned track. `PickFiles` and
/// `WindowAudioDrop` are entry-point messengers — `PickFiles` opens the
/// OS dialog (no state change until `ImportFilesToPool` fires back) and
/// `WindowAudioDrop` resolves to `ImportAndPlace` inside the handler —
/// so both are classified `UndoAction::Skip`.
#[derive(Debug, Clone)]
pub enum PoolMessage {
    /// Open the OS multi-file audio picker (the "Import audio…" chrome
    /// button, ba todo #608). The picked paths come back as a
    /// `ImportFilesToPool` message via `Task::perform`; cancelling the
    /// dialog yields an empty path list that is silently dropped.
    /// Classified `UndoAction::Skip` — no state changes at dispatch time.
    PickFiles,
    /// An audio file was dropped onto the arrangement window from the OS
    /// (ba todo #608). The handler resolves the drop to a new audio track
    /// at the current playhead position and calls through to the shared
    /// `import()` helper. One message fires per dropped file (iced emits
    /// one `FileDropped` event per path). Classified `UndoAction::Skip`
    /// (the resulting `ImportAndPlace` that the handler re-dispatches
    /// records the actual undo entry).
    WindowAudioDrop(std::path::PathBuf),
    /// Import one or more files into the pool **without** placing a clip
    /// (the "Import audio…" dialog / pool-only path).
    ImportFilesToPool(Vec<std::path::PathBuf>),
    /// Import one or more files and place them as clips at `target` (a
    /// drop on an existing lane, or on the new-audio-track zone).
    ImportAndPlace {
        paths: Vec<std::path::PathBuf>,
        target: DropTarget,
    },
    /// Import one or more files and place them at an EXACT position on an
    /// existing track — no grid snap (control endpoint `clip.place`, doc
    /// #265).
    ///
    /// [`ImportAndPlace`](Self::ImportAndPlace) snaps the drop position to
    /// the timeline grid at the current zoom, which is right for a pointer
    /// and wrong for an API: a client that asked for a sample position
    /// would get a different one depending on how far the user happened to
    /// be zoomed in. This variant places where it was told.
    ImportAndPlaceExact {
        paths: Vec<std::path::PathBuf>,
        track_id: TrackId,
        start_sample: SamplePos,
    },
    /// Place an asset that is ALREADY in the pool as a clip, with no
    /// import step (control endpoint `clip.place`, doc #265).
    ///
    /// The GUI has no equivalent — dragging a pool row always goes through
    /// `ImportAndPlace`, which short-circuits to the same placement once
    /// it sees the file is known. A remote client placing the same
    /// one-shot forty times should not re-decode it forty times, so this
    /// skips straight to the placement. `clip_id` is allocated by the
    /// caller (the derived-clip range, as `MidiClipMessage::CreateEmptyClip`
    /// does) so the control reply can name the clip immediately.
    /// Classified `UndoAction::Record`.
    PlacePooledAsset {
        clip_id: ClipId,
        asset_id: AssetId,
        track_id: TrackId,
        start_sample: SamplePos,
    },
}

impl PoolMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // The two entry-point helpers (ba todo #608) are pure intent
            // signals — no state changes at their dispatch time — so they are
            // skipped here. `PickFiles` opens the OS dialog and returns a task
            // that fires `ImportFilesToPool` later (recorded then).
            // `WindowAudioDrop` re-dispatches `ImportAndPlace` inside the handler
            // (recorded then).
            Self::PickFiles | Self::WindowAudioDrop(..) => UndoAction::Skip,
            // Audio import + placement (doc #175, todo #598) is one undoable
            // action. Recording here — before the import command is even sent —
            // captures the pre-import project (no pool asset, no placed clip, no
            // spawned track); the asset lands asynchronously and mutates state
            // via the engine-event path, which never records undo. So one undo
            // of this single snapshot removes the whole import + placement. Both
            // the pool-only and place variants are reversible (a pool asset
            // rides the `ProjectFile` snapshot just like a clip does).
            Self::ImportFilesToPool(..)
            | Self::ImportAndPlace { .. }
            | Self::ImportAndPlaceExact { .. }
            | Self::PlacePooledAsset { .. } => UndoAction::Record,
        }
    }
}

/// Audio container extensions accepted by the import entry points (the
/// chrome button, the window file-drop subscription, and the browser
/// drag-to-timeline gesture). Shared by [`is_pool_audio_path`] and the
/// OS file-picker filter so both honour exactly the same set.
pub const POOL_AUDIO_EXTENSIONS: &[&str] = &["wav", "flac", "mp3", "ogg"];

/// True when `path` looks like an importable audio file by extension
/// (wav/flac/mp3/ogg, case-insensitive). The window file-drop subscription
/// uses this to ignore non-audio OS drops so only recognised containers
/// start an import.
pub fn is_pool_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            POOL_AUDIO_EXTENSIONS
                .iter()
                .any(|accepted| ext.eq_ignore_ascii_case(accepted))
        })
}

pub fn handle(r: &mut Resonance, message: PoolMessage) -> Task<Message> {
    match message {
        // Entry point: "Import audio…" chrome button (ba todo #608).
        // Opens the OS multi-file picker; the chosen paths come back via
        // the task as `ImportFilesToPool`. A cancel (empty list) is silently
        // dropped inside the returned task's map closure.
        PoolMessage::PickFiles => return pick_audio_files_dialog(),

        // Entry point: audio file dropped onto the arrangement window from
        // the OS (ba todo #608). One message fires per dropped file (iced
        // emits one `FileDropped` event per path). The handler resolves the
        // target to a *new* audio track at the current playhead position —
        // since OS-level drops carry no cursor coordinates — and delegates
        // to the shared `import()` helper. The resulting `AddTrack` +
        // `ImportAudioToPool` commands reach the engine exactly as a
        // browser drag-to-timeline drop would.
        //
        // Guard: check for a saved project **before** calling `resolve_target`
        // so we never allocate a track id or send `AddTrack` to the engine
        // for a file that ultimately cannot be imported. (resolve_target's
        // NewTrack branch is a side-effectful step; reversing it is awkward
        // and, as of ba todo #608, there is no undo snapshot for this arm.)
        PoolMessage::WindowAudioDrop(path) => {
            if r.io.project_path.is_none() {
                r.banners.error_message = Some(
                    "Save the project before importing audio, so imported files have a home."
                        .into(),
                );
                return Task::none();
            }
            let target = DropTarget::NewTrack {
                start_sample: r.transport.playhead,
            };
            let placement = resolve_target(r, target);
            import(r, vec![path], placement);
        }

        PoolMessage::ImportFilesToPool(paths) => {
            import(r, paths, PlacementTarget::PoolOnly);
        }
        PoolMessage::ImportAndPlace { paths, target } => {
            let placement = resolve_target(r, target);
            import(r, paths, placement);
        }

        // Control-endpoint placement (doc #265). Both variants below skip
        // `resolve_target` — its grid snap is a pointer affordance, and a
        // caller that named a position means it.
        PoolMessage::ImportAndPlaceExact {
            paths,
            track_id,
            start_sample,
        } => {
            import(
                r,
                paths,
                PlacementTarget::Track {
                    track_id,
                    start_sample,
                },
            );
        }
        PoolMessage::PlacePooledAsset {
            clip_id,
            asset_id,
            track_id,
            start_sample,
        } => {
            // Only the control endpoint sends this message; its handler
            // fails the placement job when the clip never reaches the
            // mirror. Surface the reason to the GUI too, so a dropped
            // placement is never completely silent.
            if let Err(reason) = place_pooled_asset(r, clip_id, asset_id, track_id, start_sample) {
                r.banners.error_message = Some(reason);
            }
        }
    }
    Task::none()
}

/// Place an asset already in the pool as a clip, skipping the import
/// entirely. The asset carries everything a placement needs — its
/// project-relative WAV, source path, length and thumbnail peaks — so this
/// is the tail of the import flow with the slow half removed.
///
/// A missing asset id is an `Err`, never a silent no-op: the control
/// handler pre-checks the pool, but it completes the `clip.place` job
/// after this dispatch, and a swallowed placement reported as `done`
/// carries fabricated zero geometry (the defect this signature guards
/// against). The GUI cannot reach this message at all.
fn place_pooled_asset(
    r: &mut Resonance,
    clip_id: ClipId,
    asset_id: AssetId,
    track_id: TrackId,
    start_sample: SamplePos,
) -> Result<(), String> {
    let Some(asset) = r.pool.asset(asset_id).cloned() else {
        return Err(format!(
            "asset {asset_id} is no longer in the pool; nothing was placed"
        ));
    };
    crate::engine_events::pool::place_clip_with_id(
        r,
        clip_id,
        asset_id,
        &asset.project_relative_path,
        &asset.original_path,
        start_sample,
        asset.duration_frames,
        asset.thumbnail_peaks.clone(),
        track_id,
    );
    Ok(())
}

/// Open the OS multi-file audio picker. The resolved paths (or an empty
/// `Vec` on cancel) come back as [`PoolMessage::ImportFilesToPool`].
fn pick_audio_files_dialog() -> Task<Message> {
    Task::perform(
        async move {
            rfd::AsyncFileDialog::new()
                .set_title("Import Audio")
                .add_filter("Audio files", POOL_AUDIO_EXTENSIONS)
                .pick_files()
                .await
                .map(|files| {
                    files
                        .into_iter()
                        .map(|fh| fh.path().to_path_buf())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        },
        |paths| Message::Pool(PoolMessage::ImportFilesToPool(paths)),
    )
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
            let track_id = r.allocate_track_id();
            let name = new_audio_track_name(r);
            let _ = r.engine.send(AudioCommand::AddTrack {
                id: track_id,
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
        r.banners.error_message =
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
            // The import's own undo entry was recorded before dispatch.
            history_depth: r.undo.undo_len(),
        });
    }

    let _ = r.engine.send(AudioCommand::ImportAudioToPool {
        paths: path_strings,
    });

    // Open (or re-open) the transcode-progress modal. Clearing the tracker
    // first means a second import gesture replaces the previous batch's
    // status rows, so the modal always shows the current import's progress
    // rather than stale entries from a prior run.
    r.import_progress.clear();
    r.import_progress_modal_open = true;
}

/// Snap a raw drop sample position to the timeline grid, reusing the exact
/// helper the clip-drag reducer uses so a dropped clip lands on the same
/// boundaries a dragged one would.
fn snap_drop_sample(r: &Resonance, raw: SamplePos) -> SamplePos {
    crate::view::timeline::snap_sample_to_grid_tempo(
        raw,
        r.transport.bpm,
        r.transport.time_sig_num,
        r.sample_rate,
        r.viewport.zoom,
        &r.tempo_map,
    )
}

/// A unique default name for a track spawned by a new-track drop, e.g.
/// `"Audio 3"`. Counts existing tracks so repeated drops don't collide on
/// one label.
fn new_audio_track_name(r: &Resonance) -> String {
    format!("Audio {}", r.registry.tracks.len() + 1)
}
