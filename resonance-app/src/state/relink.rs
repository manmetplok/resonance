//! Transient state for the missing-file relink flow (doc #175, todo
//! #600).
//!
//! The durable fact "this asset's backing WAV is gone" lives on the pool
//! asset itself ([`PoolAsset::missing`](crate::state::pool::PoolAsset::missing),
//! set at load time by `restore_pool_assets`). This module holds only the
//! *session* state around resolving those files: which assets are
//! currently being re-imported (so the UI can show progress and a second
//! click can't double-import), and the last relink failure to surface.
//!
//! None of this is persisted or undoable — it mirrors the reference
//! panel's transient `last_error` / in-flight bookkeeping. The actual
//! relink (clearing the missing flag, re-copying the WAV, reloading the
//! clips) rides the normal project-snapshot / replay path, so *that* part
//! is undoable; see `update::relink`.

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use resonance_audio::types::{AssetId, ClipId};

/// Shared between the UI and a background "Search a folder…" walk
/// (review VIEW-29 / UPD-10): the walk bumps `dirs_scanned` as it goes
/// (the modal shows it) and stops at the next directory once `cancel`
/// is set.
#[derive(Debug, Default)]
pub struct ScanControl {
    pub cancel: AtomicBool,
    pub dirs_scanned: AtomicUsize,
}

impl ScanControl {
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn dirs_scanned(&self) -> usize {
        self.dirs_scanned.load(Ordering::Relaxed)
    }
}

/// A batch-relink folder walk running on a worker thread.
#[derive(Debug, Clone)]
pub struct RelinkScan {
    /// Matches the walk's `ScanFinished` result to this scan; a result
    /// whose token doesn't match (cancelled, superseded) is dropped.
    pub token: u64,
    pub folder: PathBuf,
    pub control: Arc<ScanControl>,
}

/// Session-level relink bookkeeping held on [`crate::Resonance`].
#[derive(Debug, Clone, Default)]
pub struct RelinkState {
    /// Assets whose replacement file is currently being copied/transcoded
    /// back into the project on a worker thread. An asset is inserted when
    /// its relink import starts and removed when the import finishes
    /// (success or failure). Lets the browser show a spinner and lets the
    /// handler ignore a duplicate relink request for an in-flight asset.
    pub in_flight: HashSet<AssetId>,
    /// The most recent relink failure, if any — a user-facing string shown
    /// until the next relink attempt clears it. Batch relinks keep only the
    /// last failure (they're independent; one bad file never aborts the
    /// rest).
    pub last_error: Option<String>,
    /// Whether the missing-files relink modal is currently on screen (doc
    /// #175, todo #607). Set automatically when a project loads with
    /// missing assets, and toggled by the Pool `relink` chip
    /// ([`RelinkMessage::ShowModal`]) / the modal's dismiss action
    /// ([`RelinkMessage::DismissModal`]). Purely presentational — not
    /// persisted, not undoable.
    ///
    /// [`RelinkMessage::ShowModal`]: crate::message::RelinkMessage::ShowModal
    /// [`RelinkMessage::DismissModal`]: crate::message::RelinkMessage::DismissModal
    pub modal_open: bool,
    /// The assets the open modal is tracking, captured when it opened (the
    /// set that was missing at that moment), in import order. The modal
    /// renders one row per id and derives per-row progress from the live
    /// pool: an id no longer flagged missing shows as *relinked*, an
    /// in-flight id shows a spinner, the rest offer `Locate…`. Keeping the
    /// original set (rather than re-reading `missing_assets()` each frame)
    /// lets the modal show the just-relinked rows as resolved instead of
    /// making them vanish, and drives the "N of M relinked" counter.
    pub modal_targets: Vec<AssetId>,
    /// The "Search a folder…" walk in flight, if any. Only one runs at a
    /// time; the imports it finds start when its result arrives.
    pub scan: Option<RelinkScan>,
    /// Token source for [`RelinkScan::token`].
    pub next_scan_token: u64,
    /// Audio clips whose own WAV (`audio/clip_<id>.wav`) was not on disk
    /// when a load or restore asked the engine for it (W4). Such a clip is
    /// kept in the timeline but plays nothing; the relink modal offers a
    /// per-clip `Locate…` for it. Re-derived on every load/restore, so it
    /// is never persisted; an id whose clip is gone is simply ignored.
    pub missing_clips: BTreeSet<ClipId>,
    /// Missing clips whose replacement file is being imported.
    pub clips_in_flight: HashSet<ClipId>,
    /// The missing clips the open modal tracks, captured when it opened —
    /// the clip counterpart of [`Self::modal_targets`].
    pub modal_clip_targets: Vec<ClipId>,
}

impl RelinkState {
    /// True when a relink import for `asset_id` is currently running.
    pub fn is_in_flight(&self, asset_id: AssetId) -> bool {
        self.in_flight.contains(&asset_id)
    }

    /// True when any relink import is currently running.
    pub fn any_in_flight(&self) -> bool {
        !self.in_flight.is_empty() || !self.clips_in_flight.is_empty()
    }

    /// True when a relink import for the missing clip `clip_id` is running.
    pub fn is_clip_in_flight(&self, clip_id: ClipId) -> bool {
        self.clips_in_flight.contains(&clip_id)
    }

    /// Open the relink modal tracking `targets` (the currently-missing
    /// assets). A no-op-safe helper: with an empty `targets` the modal
    /// view guard keeps it hidden.
    pub fn open_modal(&mut self, targets: Vec<AssetId>) {
        self.modal_open = true;
        self.modal_targets = targets;
    }

    /// Close the relink modal and forget which assets it was tracking.
    /// A folder search in flight is cancelled with it.
    pub fn close_modal(&mut self) {
        self.modal_open = false;
        self.modal_targets.clear();
        self.modal_clip_targets.clear();
        self.cancel_scan();
    }

    /// Stop the folder search in flight (if any) and forget it, so its
    /// result — whenever the worker notices — is ignored.
    pub fn cancel_scan(&mut self) {
        if let Some(scan) = self.scan.take() {
            scan.control.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// True while a "Search a folder…" walk is running.
    pub fn scanning(&self) -> bool {
        self.scan.is_some()
    }
}
