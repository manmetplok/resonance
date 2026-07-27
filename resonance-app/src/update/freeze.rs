//! Track-freeze update handlers (ba todo #574).
//!
//! Each [`FreezeMessage`] turns user intent into the matching engine
//! command (ba todo #572) and sets the *initiating* app-side
//! [`FreezeStatus`](crate::state::FreezeStatus). The engine owns the
//! offline render; its `FreezeProgress` / `FreezeCompleted` / `FreezeError`
//! / `FreezeCancelled` events (mirrored by ba todo #575) drive the later
//! `Freezing → Frozen` / `Failed` transitions and advance the batch queue.
//!
//! "Freeze all" / "freeze selected" run one track at a time: the offline
//! renderer shares plugin instances with the live mixer, so concurrent
//! renders would interleave `process()` calls. [`FreezeQueue`] holds the
//! tracks still waiting plus the completed/total counter the progress
//! overlay shows as "N / M".
//!
//! Undo: freeze / unfreeze are atomic undo entries (see `undo.rs`); the
//! rendered cache is deliberately *not* part of undo history. On restore,
//! [`Resonance::apply_freeze_restore`] detaches + deletes the cache of any
//! track that is no longer frozen and downgrades a re-frozen track to
//! `Stale` when its cache file is gone.

use std::collections::VecDeque;
use std::path::PathBuf;

use iced::Task;
use resonance_audio::types::{AudioCommand, TrackId, TrackType};

use crate::message::{FreezeMessage, Message};
use crate::state::{FreezeQueue, FreezeStatus, TrackState};
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: FreezeMessage) -> Task<Message> {
    match m {
        FreezeMessage::FreezeTrack(track_id) => {
            freeze_one(r, track_id);
        }
        FreezeMessage::UnfreezeTrack(track_id) => {
            unfreeze_one(r, track_id);
        }
        FreezeMessage::RefreezeTrack(track_id) => {
            // Re-render in place. Only meaningful when the track currently
            // carries a (typically stale) cache; otherwise treat it as a
            // fresh freeze.
            freeze_one(r, track_id);
        }
        FreezeMessage::CancelFreeze => {
            cancel_freeze(r);
        }
        FreezeMessage::FreezeSelectedTracks => {
            let tracks = selected_freezable_tracks(r);
            start_batch(r, tracks);
        }
        FreezeMessage::FreezeAllTracks => {
            let tracks = freezable_tracks(r);
            start_batch(r, tracks);
        }
    }
    Task::none()
}

/// Freeze a single track: validate, switch its status to `Freezing`, and
/// fire the engine render. Surfaces a user-facing error (and leaves the
/// status unchanged) when the track can't be frozen.
fn freeze_one(r: &mut Resonance, track_id: TrackId) {
    let Some(track) = r.registry.tracks.iter().find(|t| t.id == track_id) else {
        r.error_message = Some("Freeze: track not found".into());
        return;
    };
    if let Err(msg) = freezable(track) {
        r.error_message = Some(msg.into());
        return;
    }
    if r.transport.playing {
        r.error_message = Some("Stop transport before freezing".into());
        return;
    }
    if r.freeze.status(track_id).is_freezing() {
        // Already rendering — ignore the repeat request.
        return;
    }
    start_freeze(r, track_id);
}

/// Unfreeze a single track: detach the cache from the engine, delete the
/// cache file, and restore live editing. No-op unless the track is frozen.
fn unfreeze_one(r: &mut Resonance, track_id: TrackId) {
    if !r.freeze.status(track_id).is_frozen() {
        return;
    }
    detach_and_delete_cache(r, track_id);
    r.freeze.set(track_id, FreezeStatus::Idle);
}

/// Cancel the in-flight freeze render and abandon any active batch. The
/// engine removes the partially-written cache file before emitting
/// `FreezeCancelled`; here we just roll the optimistic UI state back.
fn cancel_freeze(r: &mut Resonance) {
    let _ = r.engine.send(AudioCommand::CancelFreeze);
    // Roll every track that this run had pushed into `Freezing` back to
    // idle. The batch's still-pending tracks never started, so they're
    // already idle.
    let freezing: Vec<TrackId> = r
        .freeze
        .statuses
        .iter()
        .filter(|(_, s)| s.is_freezing())
        .map(|(id, _)| *id)
        .collect();
    for id in freezing {
        r.freeze.set(id, FreezeStatus::Idle);
    }
    r.freeze.queue = None;
}

/// Kick off a sequential freeze batch, starting the first track now and
/// queueing the rest. No-op when nothing is freezable.
fn start_batch(r: &mut Resonance, tracks: Vec<TrackId>) {
    if r.transport.playing {
        r.error_message = Some("Stop transport before freezing".into());
        return;
    }
    // Skip tracks already frozen or mid-render — re-freezing them would be
    // redundant work the user didn't ask for in a bulk freeze.
    let queueable: VecDeque<TrackId> = tracks
        .into_iter()
        .filter(|id| matches!(r.freeze.status(*id), FreezeStatus::Idle | FreezeStatus::Failed { .. }))
        .collect();
    let Some(queue) = FreezeQueue::new(queueable) else {
        return;
    };
    let first = queue.current.expect("FreezeQueue::new always sets current");
    r.freeze.queue = Some(queue);
    if !start_freeze(r, first) {
        // The first track failed to start (e.g. unsaved project); abandon
        // the batch so a stuck queue doesn't gate the UI.
        r.freeze.queue = None;
    }
}

/// Advance the active batch to the next track after the current one
/// finished (called by the engine freeze-event mirror, ba todo #575).
/// Returns `true` when a next freeze was started, `false` when the batch
/// is exhausted (and the queue is dropped).
pub(crate) fn advance_freeze_queue(r: &mut Resonance) -> bool {
    let Some(queue) = r.freeze.queue.as_mut() else {
        return false;
    };
    match queue.advance() {
        Some(next) => {
            if start_freeze(r, next) {
                true
            } else {
                // Couldn't start the next one; keep draining so the batch
                // doesn't wedge.
                advance_freeze_queue(r)
            }
        }
        None => {
            r.freeze.queue = None;
            false
        }
    }
}

/// Compute the cache path, ensure the freeze directory exists, mark the
/// track `Freezing`, and send the render command. Returns `false` (and
/// records a `Failed` status / error) when the cache path can't be
/// established — chiefly an unsaved project.
fn start_freeze(r: &mut Resonance, track_id: TrackId) -> bool {
    let Some(dir) = freeze_dir(r) else {
        r.error_message = Some("Save the project before freezing".into());
        r.freeze.set(
            track_id,
            FreezeStatus::Failed {
                message: "Project must be saved before freezing".into(),
            },
        );
        return false;
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        let message = format!("Could not create freeze cache directory: {e}");
        r.error_message = Some(message.clone());
        r.freeze.set(track_id, FreezeStatus::Failed { message });
        return false;
    }
    let cache_path = dir.join(cache_filename(track_id));
    r.freeze
        .set(track_id, FreezeStatus::Freezing { fraction: 0.0 });
    let _ = r.engine.send(AudioCommand::FreezeTrack {
        track_id,
        cache_path: cache_path.to_string_lossy().into_owned(),
    });
    true
}

/// Detach a frozen track's cache from the engine and remove the cache file
/// from disk. Used by unfreeze and by the undo-restore reconciliation.
fn detach_and_delete_cache(r: &mut Resonance, track_id: TrackId) {
    let _ = r.engine.send(AudioCommand::UnfreezeTrack { track_id });
    if let (Some(dir), Some(name)) = (
        freeze_dir(r),
        r.freeze
            .status(track_id)
            .cache_ref()
            .map(|c| c.cache_filename.clone()),
    ) {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

/// The project's freeze-cache directory (`<project>.rproj/freeze/`), or
/// `None` when no project has been saved yet.
fn freeze_dir(r: &Resonance) -> Option<PathBuf> {
    r.io.project_path.as_ref().map(|p| p.join("freeze"))
}

/// Cache filename for a track, relative to the freeze directory. Matches
/// the basename the engine records in the returned `FreezeCacheRef`.
fn cache_filename(track_id: TrackId) -> String {
    format!("freeze_{track_id}.wav")
}

/// A track can be frozen when it has a live render to capture: instrument
/// and vocal tracks that aren't sub-tracks (sub-tracks are frozen as part
/// of their parent's render). Audio tracks have no synth to freeze.
fn freezable(track: &TrackState) -> Result<(), &'static str> {
    if track.sub_track.is_some() {
        return Err("Freeze the parent track to capture its sub-tracks, not a sub-track itself");
    }
    match track.track_type {
        TrackType::Instrument | TrackType::Vocal => Ok(()),
        TrackType::Audio => Err("Freeze is only available on instrument and vocal tracks"),
    }
}

/// All freezable tracks in display order.
fn freezable_tracks(r: &Resonance) -> Vec<TrackId> {
    r.sorted_tracks()
        .iter()
        .filter(|t| freezable(t).is_ok())
        .map(|t| t.id)
        .collect()
}

/// The selected track(s) that are freezable. The app currently models a
/// single track selection, so this yields at most one id; the batch path
/// still works unchanged once multi-select lands.
fn selected_freezable_tracks(r: &Resonance) -> Vec<TrackId> {
    r.interaction
        .selected_track
        .and_then(|id| r.registry.tracks.iter().find(|t| t.id == id))
        .filter(|t| freezable(t).is_ok())
        .map(|t| vec![t.id])
        .unwrap_or_default()
}

impl Resonance {
    /// Reconcile freeze state after an undo/redo restore drove the project
    /// back to `target`. The rendered cache is not part of undo history, so:
    ///
    /// - a track that was frozen but is idle in `target` (undo of a freeze)
    ///   has its cache detached from the engine and deleted from disk;
    /// - a track that becomes frozen in `target` (redo of a freeze) keeps
    ///   that status only if its cache file still exists, otherwise it is
    ///   downgraded to `Stale` (the cache was removed by the matching undo).
    ///
    /// Any in-flight batch is abandoned — a restore stops the engine.
    pub(crate) fn apply_freeze_restore(
        &mut self,
        target: std::collections::HashMap<TrackId, FreezeStatus>,
    ) {
        // Detach + delete caches for tracks that are no longer frozen.
        let no_longer_frozen: Vec<TrackId> = self
            .freeze
            .statuses
            .iter()
            .filter(|(id, status)| {
                status.is_frozen() && !target.get(id).is_some_and(FreezeStatus::is_frozen)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in no_longer_frozen {
            detach_and_delete_cache(self, id);
        }

        // Apply the target, downgrading any restored-frozen track whose
        // cache file is gone to `Stale`.
        let dir = freeze_dir(self);
        let reconciled = target
            .into_iter()
            .map(|(id, status)| {
                let resolved = match &status {
                    FreezeStatus::Frozen { cache_ref } => {
                        let exists = dir
                            .as_ref()
                            .is_some_and(|d| d.join(&cache_ref.cache_filename).exists());
                        if exists {
                            status
                        } else {
                            FreezeStatus::Stale {
                                cache_ref: cache_ref.clone(),
                            }
                        }
                    }
                    _ => status,
                };
                (id, resolved)
            })
            .collect();
        self.freeze.statuses = reconciled;
        self.freeze.queue = None;
    }
}
