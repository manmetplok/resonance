//! Transient state for the missing-file relink flow (doc #175, todo
//! #600).
//!
//! The durable fact "this asset's backing WAV is gone" lives on the pool
//! asset itself ([`PoolAsset::missing`](crate::state::pool::PoolAsset::missing),
//! set at load time by `restore_pool`). This module holds only the
//! *session* state around resolving those files: which assets are
//! currently being re-imported (so the UI can show progress and a second
//! click can't double-import), and the last relink failure to surface.
//!
//! None of this is persisted or undoable — it mirrors the reference
//! panel's transient `last_error` / in-flight bookkeeping. The actual
//! relink (clearing the missing flag, re-copying the WAV, reloading the
//! clips) rides the normal project-snapshot / replay path, so *that* part
//! is undoable; see `update::relink`.

use std::collections::HashSet;

use resonance_audio::types::AssetId;

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
}

impl RelinkState {
    /// True when a relink import for `asset_id` is currently running.
    pub fn is_in_flight(&self, asset_id: AssetId) -> bool {
        self.in_flight.contains(&asset_id)
    }

    /// True when any relink import is currently running.
    pub fn any_in_flight(&self) -> bool {
        !self.in_flight.is_empty()
    }

    /// Open the relink modal tracking `targets` (the currently-missing
    /// assets). A no-op-safe helper: with an empty `targets` the modal
    /// view guard keeps it hidden.
    pub fn open_modal(&mut self, targets: Vec<AssetId>) {
        self.modal_open = true;
        self.modal_targets = targets;
    }

    /// Close the relink modal and forget which assets it was tracking.
    pub fn close_modal(&mut self) {
        self.modal_open = false;
        self.modal_targets.clear();
    }
}
