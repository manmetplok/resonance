//! Engine freeze-event mirror (ba todo #575).
//!
//! The engine renders a track's post-FX output to a freeze-cache WAV on a
//! worker thread (ba todo #571/#572) and reports back through the
//! `Freeze*` [`AudioEvent`](resonance_audio::types::AudioEvent) family.
//! This module folds those events into the app's per-track
//! [`FreezeStatus`](crate::state::FreezeStatus) and advances the batch
//! [`FreezeQueue`](crate::state::FreezeQueue) so a "freeze all" / "freeze
//! selected" run drives itself purely from engine events:
//!
//! - [`progress`] updates the in-flight `Freezing { fraction }`.
//! - [`completed`] stores the [`FreezeCacheRef`] and marks the track
//!   `Frozen`, then starts the next queued track (if any).
//! - [`error`] marks the track `Failed` (playback falls back to the live
//!   chain) and still advances the batch so one bad track can't wedge it.
//! - [`cancelled`] returns the track to live (`Idle`) with no cache and
//!   abandons the whole batch — cancel stops the run, it doesn't skip.
//!
//! Attaching the *decoded* cache buffer to the engine for playback
//! (`AudioCommand::SetTrackFrozenSource`) is owned by the project-load
//! rehydrate path (ba todo #577); a fresh render keeps the buffer the
//! engine already holds, so the mirror here only tracks app-side state.

use resonance_audio::types::TrackId;
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

use crate::state::FreezeStatus;
use crate::update::freeze::advance_freeze_queue;
use crate::Resonance;

/// `AudioEvent::FreezeProgress` — update the in-flight render fraction.
///
/// Only applied while the track is actually `Freezing`; a late progress
/// event that races past a `Completed`/`Cancelled`/`Error` terminal must
/// not resurrect the track into `Freezing`.
pub(crate) fn progress(r: &mut Resonance, track_id: TrackId, fraction: f32) {
    if r.freeze.status(track_id).is_freezing() {
        let fraction = fraction.clamp(0.0, 1.0);
        r.freeze
            .set(track_id, FreezeStatus::Freezing { fraction });
    }
}

/// `AudioEvent::FreezeCompleted` — store the cache ref, mark the track
/// `Frozen`, and advance the batch to the next track.
pub(crate) fn completed(r: &mut Resonance, track_id: TrackId, cache_ref: FreezeCacheRef) {
    let mut cache_ref = cache_ref;
    // The engine may report the ref with whatever status it rendered with;
    // a successful completion is canonically `Frozen`.
    cache_ref.status = FreezeCacheStatus::Frozen;
    r.freeze.set(track_id, FreezeStatus::Frozen { cache_ref });
    advance_batch_if_current(r, track_id);
}

/// `AudioEvent::FreezeError` — mark the track `Failed` and fall back to
/// live playback (a `Failed` status carries no cache, so the mixer runs
/// the live chain). The batch still advances so a single failure doesn't
/// strand the remaining tracks.
pub(crate) fn error(r: &mut Resonance, track_id: TrackId, message: String) {
    r.freeze.set(track_id, FreezeStatus::Failed { message });
    advance_batch_if_current(r, track_id);
}

/// `AudioEvent::FreezeCancelled` — return the track to live editing with
/// no cache and abandon the active batch. The engine has already removed
/// the partially-written cache file before emitting this.
pub(crate) fn cancelled(r: &mut Resonance, track_id: TrackId) {
    r.freeze.set(track_id, FreezeStatus::Idle);
    // Cancel stops the whole run rather than skipping to the next track:
    // mirror `update::freeze::cancel_freeze` and drop the queue. (It may
    // already be `None` when the user-initiated cancel cleared it
    // optimistically; this is idempotent.)
    r.freeze.queue = None;
}

/// Advance the batch queue when `track_id` is the one currently rendering.
/// Single (non-batch) freezes carry no queue, so this is a no-op for them.
fn advance_batch_if_current(r: &mut Resonance, track_id: TrackId) {
    let is_current = r.freeze.queue.as_ref().and_then(|q| q.current) == Some(track_id);
    if is_current {
        advance_freeze_queue(r);
    }
}
