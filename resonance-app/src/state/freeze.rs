//! App-side track-freeze orchestration state (ba todo #574).
//!
//! Freezing renders a CPU-heavy instrument/vocal track's post-FX output
//! to a cache WAV so the live synth + plugin chain can be skipped during
//! playback. This module holds the *app's* view of that lifecycle: the
//! per-track UI status machine and the batch queue that drives
//! "freeze selected" / "freeze all".
//!
//! The engine owns the actual render (ba todo #571/#572); the app issues
//! the [`AudioCommand::FreezeTrack`](resonance_audio::types::AudioCommand)
//! and mirrors the resulting progress / completion events back into the
//! [`FreezeStatus`] here (the event mirror is ba todo #575). The persisted
//! freeze metadata uses the shared [`resonance_common`] types so the
//! project-IO slice (ba todo #577) can serialise it unchanged.

use std::collections::HashMap;
use std::collections::VecDeque;

use resonance_audio::types::TrackId;
use resonance_common::{FreezeCacheRef, FreezeCacheStatus, TrackFreezeState};

/// Runtime UI status for a single track's freeze.
///
/// Absence of an entry in [`FreezeState::statuses`] is equivalent to
/// [`FreezeStatus::Idle`] — a track is "not frozen, editing live" by
/// default, so the map only ever holds non-idle tracks.
#[derive(Debug, Clone, PartialEq)]
pub enum FreezeStatus {
    /// Not frozen — the live synth + FX chain plays, the track is editable.
    Idle,
    /// An offline freeze render is in flight. `fraction` is in `[0.0, 1.0]`,
    /// fed from the engine's `FreezeProgress` events (ba todo #575).
    Freezing { fraction: f32 },
    /// Frozen with a valid cache attached; playback reads the cache.
    Frozen { cache_ref: FreezeCacheRef },
    /// Frozen, but the inputs changed since the cache was rendered, so it
    /// no longer matches the live track. Needs a refreeze. Carries the
    /// (now-stale) cache so the user can still play it until refrozen.
    Stale { cache_ref: FreezeCacheRef },
    /// The last freeze attempt failed; carries a user-facing message.
    Failed { message: String },
}

impl FreezeStatus {
    /// True while an offline render is in flight for this track.
    pub fn is_freezing(&self) -> bool {
        matches!(self, FreezeStatus::Freezing { .. })
    }

    /// True when the track has a cache (valid or stale) attached — i.e. it
    /// should play from the cache rather than the live chain.
    pub fn is_frozen(&self) -> bool {
        matches!(self, FreezeStatus::Frozen { .. } | FreezeStatus::Stale { .. })
    }

    /// The attached cache reference, if any (present for `Frozen`/`Stale`).
    pub fn cache_ref(&self) -> Option<&FreezeCacheRef> {
        match self {
            FreezeStatus::Frozen { cache_ref } | FreezeStatus::Stale { cache_ref } => {
                Some(cache_ref)
            }
            _ => None,
        }
    }

    /// Project the runtime status onto the persisted [`TrackFreezeState`]
    /// shape the project-IO slice (ba todo #577) serialises. Transient
    /// states (`Idle`, `Freezing`, `Failed`) round-trip as "not frozen".
    pub fn to_persisted(&self) -> TrackFreezeState {
        match self {
            FreezeStatus::Frozen { cache_ref } => {
                let mut cr = cache_ref.clone();
                cr.status = FreezeCacheStatus::Frozen;
                TrackFreezeState::frozen(cr)
            }
            FreezeStatus::Stale { cache_ref } => {
                let mut cr = cache_ref.clone();
                cr.status = FreezeCacheStatus::Stale;
                TrackFreezeState::frozen(cr)
            }
            FreezeStatus::Idle | FreezeStatus::Freezing { .. } | FreezeStatus::Failed { .. } => {
                TrackFreezeState::unfrozen()
            }
        }
    }
}

/// A sequential "freeze selected" / "freeze all" batch.
///
/// Freezes run one track at a time (the offline renderer shares plugin
/// instances with the live mixer, so they can't overlap). The queue holds
/// the tracks still waiting plus a completed/total counter that the
/// progress overlay renders as "N / M".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FreezeQueue {
    /// Tracks still waiting to be frozen, in order. Excludes the track
    /// currently rendering (which lives in `current`).
    pub pending: VecDeque<TrackId>,
    /// The track currently rendering, if any.
    pub current: Option<TrackId>,
    /// How many tracks in this batch have finished (success or skip).
    pub completed: usize,
    /// Total tracks in the batch — the "M" in the "N / M" counter.
    pub total: usize,
}

impl FreezeQueue {
    /// Build a batch from `tracks`, taking the first as the one to start
    /// rendering now. Returns `None` (and an empty queue) when `tracks` is
    /// empty.
    pub fn new(mut tracks: VecDeque<TrackId>) -> Option<Self> {
        let total = tracks.len();
        let current = tracks.pop_front()?;
        Some(Self {
            pending: tracks,
            current: Some(current),
            completed: 0,
            total,
        })
    }

    /// Mark the current track done and advance to the next, returning it.
    /// Returns `None` when the batch is exhausted (the caller should then
    /// drop the queue).
    pub fn advance(&mut self) -> Option<TrackId> {
        if self.current.is_some() {
            self.completed += 1;
        }
        self.current = self.pending.pop_front();
        self.current
    }
}

/// All app-side freeze state: per-track UI status plus the active batch
/// queue. Held on [`Resonance`](crate::Resonance) and cleared on project
/// load (freeze status is rebuilt from the project file by ba todo #577,
/// and the queue never crosses a load boundary).
#[derive(Debug, Clone, Default)]
pub struct FreezeState {
    /// Non-idle freeze statuses, keyed by track. A missing entry means
    /// [`FreezeStatus::Idle`].
    pub statuses: HashMap<TrackId, FreezeStatus>,
    /// The in-flight "freeze selected" / "freeze all" batch, if any.
    pub queue: Option<FreezeQueue>,
}

impl FreezeState {
    /// The status for `track_id` — [`FreezeStatus::Idle`] when no entry
    /// exists. Returns an owned value so callers don't fight the borrow
    /// checker over the map; statuses are cheap to clone.
    pub fn status(&self, track_id: TrackId) -> FreezeStatus {
        self.statuses
            .get(&track_id)
            .cloned()
            .unwrap_or(FreezeStatus::Idle)
    }

    /// Set a track's status, collapsing `Idle` to a map removal so the map
    /// only ever holds non-idle tracks.
    pub fn set(&mut self, track_id: TrackId, status: FreezeStatus) {
        if status == FreezeStatus::Idle {
            self.statuses.remove(&track_id);
        } else {
            self.statuses.insert(track_id, status);
        }
    }

    /// Reset a track to idle (removes its entry).
    pub fn clear(&mut self, track_id: TrackId) {
        self.statuses.remove(&track_id);
    }

    /// True while any freeze (single or batched) is rendering. Used to gate
    /// transport / mutating UI the same way the bounce-in-progress flag does.
    pub fn any_in_flight(&self) -> bool {
        self.queue.is_some() || self.statuses.values().any(FreezeStatus::is_freezing)
    }

    /// Drop all freeze state — called on project load.
    pub fn reset(&mut self) {
        self.statuses.clear();
        self.queue = None;
    }
}
