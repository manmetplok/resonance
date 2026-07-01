//! Transient orchestration state for the import → placement flow
//! (doc #175, ba todo #598) and per-file import-progress tracking
//! (ba todo #597).
//!
//! A multi-file import (dialog or drop) fans out into one
//! `AudioCommand::ImportAudioToPool` and, per file, an ordered lifecycle
//! of engine events (`ImportProgress` → `AssetImported` / `ImportFailed`).
//! The app doesn't know a file's engine-assigned `AssetId` at send time —
//! only its source path — so this side-table remembers, per queued source
//! file, *what to do once its asset lands*: place it as a clip on a
//! target track at a sample position, or nothing (a pool-only import).
//! When `AssetImported` arrives the handler matches back by the original
//! source path, performs the placement, and drops the entry.
//!
//! This is **transient runtime state**, deliberately not part of
//! `ProjectFile` and not captured by the undo snapshot: it only exists
//! between issuing an import and the engine's asset events landing. The
//! *result* of the flow — the pool asset and the placed clip — is what
//! rides persistence and undo (both are `ProjectFile` facts, doc #175 /
//! ba todo #596). The single-action undo is recorded up front, when the
//! import is initiated (see `undo::classify`), capturing the pre-import
//! project so one undo removes the whole import + placement.

use resonance_audio::types::{AssetId, SamplePos, TrackId};

/// What to do with an imported asset once the engine reports it landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementTarget {
    /// Import into the pool only — do not place a clip. The dialog
    /// "Import audio…" path and any pool-only add.
    PoolOnly,
    /// Place the asset as an audio clip on `track_id` at `start_sample`
    /// (already grid-snapped by the orchestration). Used both for a drop
    /// on an existing lane and — after the new lane's id is reserved and
    /// its `AddTrack` issued — for a drop on the new-audio-track zone.
    Track {
        track_id: TrackId,
        start_sample: SamplePos,
    },
}

/// One queued source file awaiting its `AssetImported` event, plus what
/// to do with it when it arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingImport {
    /// Source path exactly as passed in `AudioCommand::ImportAudioToPool`
    /// — the key the engine echoes back as `original_path`.
    pub source_path: String,
    /// Placement to perform once the asset lands.
    pub target: PlacementTarget,
}

/// In-flight import placements, keyed by source path. Emptied as each
/// file's `AssetImported` (or `ImportFailed`) event is handled.
#[derive(Debug, Clone, Default)]
pub struct PendingImports {
    entries: Vec<PendingImport>,
}

impl PendingImports {
    /// Queue a placement for a source file about to be imported.
    pub fn push(&mut self, entry: PendingImport) {
        self.entries.push(entry);
    }

    /// Take the placement queued for `source_path`, removing it. Matches
    /// the *first* queued entry for the path (a file imported twice in
    /// one gesture resolves in FIFO order). `None` when nothing is queued
    /// for the path — e.g. a stray `AssetImported` for a re-import or an
    /// asset that arrived after its entry was already consumed.
    pub fn take_matching(&mut self, source_path: &str) -> Option<PlacementTarget> {
        let pos = self
            .entries
            .iter()
            .position(|e| e.source_path == source_path)?;
        Some(self.entries.remove(pos).target)
    }

    /// True when no placements are pending.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of queued placements — used by tests and diagnostics.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

// ---------------------------------------------------------------------------
// Per-file import-progress tracking (ba todo #597)
// ---------------------------------------------------------------------------

/// Progress status of a single file in an audio-import batch.
///
/// `Queued` / `Working` / `Done` map directly to the engine's
/// [`resonance_audio::types::ImportStage`] variants; `Failed` is the
/// terminal error case that arrives via [`AudioEvent::ImportFailed`]
/// rather than `ImportProgress`. Displayed by the transcode-progress
/// modal (doc #175, todo #606).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileImportProgress {
    /// File is in the import queue but the engine hasn't started it yet.
    Queued,
    /// The engine is actively transcoding / resampling this file.
    Working,
    /// Import succeeded; the asset is now in the pool.
    Done,
    /// Import failed with a user-facing error `reason`.
    Failed { reason: String },
}

/// One per-file progress row for the audio-import transcode modal.
/// Keyed by [`AssetId`] (set by the engine at import time and echoed
/// back via `ImportProgress` / `ImportFailed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileImportStatus {
    /// Engine-assigned id for this asset.
    pub asset_id: AssetId,
    /// Source path as passed in `AudioCommand::ImportAudioToPool`.
    pub path: String,
    /// Current progress / outcome.
    pub progress: FileImportProgress,
}

/// Transient per-file progress tracker for the audio-import transcode
/// modal (doc #175, todo #606).
///
/// Populated from `ImportProgress` and `ImportFailed` engine events
/// (mirrored in ba todo #597) while an import batch is in flight.
/// Cleared when the modal is dismissed or a new batch starts. Not
/// undoable and not persisted.
#[derive(Debug, Clone, Default)]
pub struct ImportProgressTracker {
    entries: Vec<FileImportStatus>,
}

impl ImportProgressTracker {
    /// Upsert a file's progress entry. Matches by `asset_id` and updates
    /// in place; appends when the id is not yet tracked.
    pub fn upsert(&mut self, asset_id: AssetId, path: String, progress: FileImportProgress) {
        if let Some(slot) = self.entries.iter_mut().find(|e| e.asset_id == asset_id) {
            slot.path = path;
            slot.progress = progress;
        } else {
            self.entries.push(FileImportStatus {
                asset_id,
                path,
                progress,
            });
        }
    }

    /// All tracked file statuses, in the order they first appeared.
    pub fn statuses(&self) -> &[FileImportStatus] {
        &self.entries
    }

    /// Clear all entries (e.g. when the modal is dismissed or a new
    /// batch replaces an old one).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// True when every tracked entry has reached a terminal state
    /// (`Done` or `Failed`); false while any entry is `Queued` or
    /// `Working`. Also true when the tracker is empty.
    pub fn is_complete(&self) -> bool {
        self.entries.iter().all(|e| {
            matches!(
                e.progress,
                FileImportProgress::Done | FileImportProgress::Failed { .. }
            )
        })
    }
}
