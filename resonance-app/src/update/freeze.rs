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
//! track that is no longer frozen, re-attaches the cache of a restored
//! freeze the engine does not hold, and downgrades a re-frozen track to
//! `Stale` when its cache file is gone or undecodable.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use iced::Task;
use resonance_audio::types::{AudioCommand, TrackId, TrackType};
use resonance_common::{FreezeCacheStatus, TrackFreezeState};

use crate::message::Message;
use crate::state::{FreezeQueue, FreezeStatus, MidiClipState, TrackState};
use crate::Resonance;

/// Track-freeze actions raised from the track header / context menu and
/// the Tracks header-cap "Freeze all" button. Each variant maps to one
/// engine command (or, for the batch variants, a sequence driven one
/// track at a time). The handlers set the initiating UI status
/// ([`FreezeStatus`](crate::state::FreezeStatus)) and let the engine's
/// progress / completion events (mirrored by ba todo #575) drive the
/// later transitions.
#[derive(Debug, Clone)]
pub enum FreezeMessage {
    /// Freeze one track: render its post-FX output to a cache WAV and
    /// switch playback to the cache. No-op if it's already freezing.
    FreezeTrack(TrackId),
    /// Unfreeze one track: detach the cache, remove the cache file, and
    /// restore live synth + FX editing.
    UnfreezeTrack(TrackId),
    /// Re-render a frozen (typically stale) track's cache in place.
    RefreezeTrack(TrackId),
    /// Cancel the in-flight freeze render. Also abandons any active batch.
    CancelFreeze,
    /// Freeze every currently selected freezable track, sequentially.
    FreezeSelectedTracks,
    /// Freeze every freezable track in the project, sequentially.
    FreezeAllTracks,
    /// Open the project's freeze-cache directory in the OS file manager
    /// (the context menu's "Reveal freeze cache…" entry, design doc #181).
    /// Surfaces an error when the project has never been saved (no cache
    /// directory exists yet in that case).
    RevealFreezeCache,
}

impl FreezeMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // Freeze edits. Freeze / unfreeze / refreeze / batch-freeze are
            // discrete, atomic transitions worth an undo entry; the rendered
            // cache is deliberately excluded from history (see `ProjectTrack::freeze` and
            // `apply_freeze_restore`). Cancelling an in-flight render is a
            // transient abort, not a project mutation — skip it.
            Self::CancelFreeze => UndoAction::Skip,
            // Opening the cache directory in the file manager reads state
            // only — never a project mutation (ba todo #581).
            Self::RevealFreezeCache => UndoAction::Skip,
            Self::FreezeTrack(_)
            | Self::UnfreezeTrack(_)
            | Self::RefreezeTrack(_)
            | Self::FreezeSelectedTracks
            | Self::FreezeAllTracks => UndoAction::Record,
        }
    }
}

pub fn handle(r: &mut Resonance, m: FreezeMessage) -> Task<Message> {
    // Every freeze action is reachable from the track context menu (ba
    // todo #581); acting on an entry closes the menu, and the messages
    // arriving from other surfaces (header toggle, shortcuts) are no-ops
    // on an already-`None` menu.
    r.ui.interaction.track_menu = None;
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
        FreezeMessage::RevealFreezeCache => {
            reveal_freeze_cache(r);
        }
    }
    Task::none()
}

/// Open the project's freeze-cache directory in the OS file manager (the
/// context menu's "Reveal freeze cache…" entry, ba todo #581). Creates the
/// directory first so the file manager never errors on a project that has
/// simply not frozen anything yet; an unsaved project (no cache location)
/// surfaces a user-facing error instead.
fn reveal_freeze_cache(r: &mut Resonance) {
    let Some(dir) = freeze_dir(r) else {
        r.banners.error_message = Some("Save the project before revealing the freeze cache".into());
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        r.banners.error_message = Some(format!("Could not open freeze cache directory: {e}"));
        return;
    }
    write_cache_gitignore(&dir);
    crate::update::external_instrument::reveal_path_in_file_manager(&dir);
}

/// Freeze a single track: validate, switch its status to `Freezing`, and
/// fire the engine render. Surfaces a user-facing error (and leaves the
/// status unchanged) when the track can't be frozen.
fn freeze_one(r: &mut Resonance, track_id: TrackId) {
    let Some(track) = r.registry.tracks.iter().find(|t| t.id == track_id) else {
        r.banners.error_message = Some("Freeze: track not found".into());
        return;
    };
    if let Err(msg) = freezable(track) {
        r.banners.error_message = Some(msg.into());
        return;
    }
    if r.transport.playing {
        r.banners.error_message = Some("Stop transport before freezing".into());
        return;
    }
    // An offline control measurement holds the offline renderer
    // exclusively; a freeze render on top of it would drive the same
    // live plugin instances from two renderers at once (mirrors
    // `meter.measure` refusing while a freeze runs).
    if r.offline_measure_in_progress() {
        r.banners.error_message =
            Some("A measurement is in progress; freeze again when it finishes".into());
        return;
    }
    if r.freeze.status(track_id).is_freezing() {
        // Already rendering — ignore the repeat request.
        return;
    }
    start_freeze(r, track_id);
}

/// Unfreeze a single track: detach the cache from the engine, delete the
/// cache file, and restore live editing. For a frozen (or stale-frozen)
/// track this tears down the cache; for a `Failed` track — which already fell
/// back to live with no cache attached — it just clears the failed status,
/// which is what the freeze-failed banner's "Dismiss" action drives (design
/// doc #181). No-op for `Idle` / `Freezing`.
fn unfreeze_one(r: &mut Resonance, track_id: TrackId) {
    match r.freeze.status(track_id) {
        s if s.is_frozen() => {
            detach_and_delete_cache(r, track_id);
            r.freeze.set(track_id, FreezeStatus::Idle);
        }
        FreezeStatus::Failed { .. } => {
            r.freeze.set(track_id, FreezeStatus::Idle);
        }
        _ => {}
    }
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
        r.banners.error_message = Some("Stop transport before freezing".into());
        return;
    }
    // Same offline-renderer exclusion as `freeze_one`.
    if r.offline_measure_in_progress() {
        r.banners.error_message =
            Some("A measurement is in progress; freeze again when it finishes".into());
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
        r.banners.error_message = Some("Save the project before freezing".into());
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
        r.banners.error_message = Some(message.clone());
        r.freeze.set(track_id, FreezeStatus::Failed { message });
        return false;
    }
    write_cache_gitignore(&dir);
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
    let dir = freeze_dir(r);
    detach_and_delete_cache_in(r, track_id, dir.as_deref());
}

/// [`detach_and_delete_cache`] with the freeze-cache directory given — a
/// restore passes its own (`ReconcileCtx::project_dir`).
fn detach_and_delete_cache_in(r: &mut Resonance, track_id: TrackId, dir: Option<&Path>) {
    let _ = r.engine.send(AudioCommand::UnfreezeTrack { track_id });
    if let (Some(dir), Some(name)) = (
        dir,
        r.freeze
            .status(track_id)
            .cache_ref()
            .map(|c| c.cache_filename.clone()),
    ) {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

/// The active project's freeze-cache directory, or `None` when no project
/// has been saved yet. See [`freeze_cache_dir_for`] for the layout.
fn freeze_dir(r: &Resonance) -> Option<PathBuf> {
    r.io
        .project_path
        .as_ref()
        .map(|p| freeze_cache_dir_for(p))
}

/// The freeze-cache directory for a project at `project_path` — a *sibling*
/// `<project>.freeze/` directory next to the `.rproj` bundle (e.g.
/// `Song.rproj` → `Song.freeze`), **not** a child inside it (ba todo #577).
///
/// Keeping the cache outside the project bundle means a manual save never
/// bundles the (potentially large) freeze WAVs, and the cache is a clean
/// build artifact: it can be deleted wholesale and is regenerated by a
/// refreeze. A self-ignoring `.gitignore` is dropped inside it on creation
/// so version-controlled project folders don't track the cache.
pub(crate) fn freeze_cache_dir_for(project_path: &Path) -> PathBuf {
    project_path.with_extension("freeze")
}

/// Cache filename for a track, relative to the freeze directory. Matches
/// the basename the engine records in the returned `FreezeCacheRef`.
fn cache_filename(track_id: TrackId) -> String {
    format!("freeze_{track_id}.wav")
}

/// Drop a self-ignoring `.gitignore` into the freeze-cache directory so a
/// version-controlled projects folder never tracks the rendered caches
/// (they're build artifacts — see [`freeze_cache_dir_for`]). Best-effort:
/// a write failure is silently ignored, and the file is only written once
/// (the common `ignore everything here` pattern, like Cargo's `target/`).
fn write_cache_gitignore(dir: &Path) {
    let gitignore = dir.join(".gitignore");
    if !gitignore.exists() {
        let _ = std::fs::write(
            &gitignore,
            "# Resonance freeze cache — regenerated build artifact, do not commit.\n*\n",
        );
    }
}

/// A track can be frozen when it has a live render to capture: instrument
/// and vocal tracks that aren't sub-tracks (sub-tracks are frozen as part
/// of their parent's render). Audio tracks have no synth to freeze.
/// `pub(crate)` so the track context menu (ba todo #581) can derive its
/// enabled / disabled item states from the same predicate.
pub(crate) fn freezable(track: &TrackState) -> Result<(), &'static str> {
    if track.sub_track.is_some() {
        return Err("Freeze the parent track to capture its sub-tracks, not a sub-track itself");
    }
    match track.track_type {
        TrackType::Instrument | TrackType::Vocal => Ok(()),
        TrackType::Audio => Err("Freeze is only available on instrument and vocal tracks"),
    }
}

/// All freezable tracks in display order. `pub(crate)` so the context
/// menu / header-cap button (ba todo #581) can compute enabled states.
pub(crate) fn freezable_tracks(r: &Resonance) -> Vec<TrackId> {
    r.sorted_tracks()
        .iter()
        .filter(|t| freezable(t).is_ok())
        .map(|t| t.id)
        .collect()
}

/// The selected track(s) that are freezable. The app currently models a
/// single track selection, so this yields at most one id; the batch path
/// still works unchanged once multi-select lands.
pub(crate) fn selected_freezable_tracks(r: &Resonance) -> Vec<TrackId> {
    r.ui.interaction
        .selected_track
        .and_then(|id| r.registry.tracks.iter().find(|t| t.id == id))
        .filter(|t| freezable(t).is_ok())
        .map(|t| vec![t.id])
        .unwrap_or_default()
}

impl Resonance {
    /// Re-attach persisted freeze caches after a project load (ba todo #577).
    ///
    /// `freezes` is each track's persisted [`TrackFreezeState`] paired with
    /// its id, taken from the loaded project file; `project_dir` is the
    /// loaded `.rproj` directory (used to resolve the sibling freeze-cache
    /// dir). For every track that was saved frozen:
    ///
    /// - decode its cache WAV and hand the engine the buffer via
    ///   `SetTrackFrozenSource`, so playback replays the cache with **no
    ///   re-render**, then mirror the persisted status (`Frozen` / `Stale`)
    ///   into the app freeze map; or
    /// - if the cache file is missing / corrupt / unreadable, load the track
    ///   as `Stale` with no attached source (the live chain plays until the
    ///   user refreezes) — never panicking on a bad cache.
    ///
    /// Only called on the disk-load path; undo/redo restores go through
    /// [`Self::apply_freeze_restore`] instead.
    pub(crate) fn rehydrate_frozen_tracks(
        &mut self,
        project_dir: &Path,
        freezes: &[(TrackId, TrackFreezeState)],
    ) {
        let dir = freeze_cache_dir_for(project_dir);
        for (track_id, state) in freezes {
            let track_id = *track_id;
            // Live / not-frozen tracks carry no cache to restore. A project
            // saved while a track was stale stays stale (so the UI keeps
            // offering a refreeze) but still plays the cache it has.
            let status = FreezeStatus::from_persisted(state);
            if !status.is_frozen() {
                continue;
            }
            let (status, attached) = self.attach_freeze_cache(track_id, status, Some(&dir));
            self.freeze.set(track_id, status);
            if attached {
                // The project was saved with this cache valid, so the
                // content just replayed is what it was rendered from.
                self.note_freeze_content_baseline(track_id);
            }
        }
    }

    /// Decode a frozen (or stale-frozen) track's cache from `dir` and hand
    /// the engine the buffer via `SetTrackFrozenSource`, so playback reads
    /// the cache instead of the live chain. Returns the status the track
    /// should take and whether a source was attached: `status` unchanged on
    /// success; on a missing / corrupt / unreadable cache (or no `dir`),
    /// `Stale` with nothing attached — the live chain plays until the user
    /// refreezes — and never a panic. The one attach step shared by a disk
    /// load ([`Self::rehydrate_frozen_tracks`]) and an undo/redo restore
    /// ([`Self::reconcile_freeze_statuses`], FU-A4a).
    fn attach_freeze_cache(
        &self,
        track_id: TrackId,
        status: FreezeStatus,
        dir: Option<&Path>,
    ) -> (FreezeStatus, bool) {
        let Some(cache_ref) = status.cache_ref().cloned() else {
            return (status, false);
        };
        let decoded = dir.map(|dir| {
            resonance_audio::read_freeze_cache(
                &dir.join(&cache_ref.cache_filename),
                cache_ref.clone(),
            )
        });
        match decoded {
            Some(Ok(source)) => {
                let _ = self.engine.send(AudioCommand::SetTrackFrozenSource {
                    track_id,
                    source: Some(source),
                });
                (status, true)
            }
            other => {
                match other {
                    Some(Err(e)) => tracing::warn!(
                        "Freeze cache for track {track_id} unavailable ({e}); marking stale"
                    ),
                    _ => tracing::warn!(
                        "Freeze cache for track {track_id}: no project path; marking stale"
                    ),
                }
                let mut cache_ref = cache_ref;
                cache_ref.status = FreezeCacheStatus::Stale;
                (FreezeStatus::Stale { cache_ref }, false)
            }
        }
    }

    /// Tear down a track's freeze cache when the track is deleted (ba todo
    /// #577): detach the frozen source from the engine, delete the cache
    /// WAV, and clear the app-side status. No-op for a track that was never
    /// frozen.
    pub(crate) fn cleanup_freeze_on_delete(&mut self, track_id: TrackId) {
        if !self.freeze.status(track_id).is_frozen() {
            return;
        }
        detach_and_delete_cache(self, track_id);
        self.freeze.clear(track_id);
    }

    /// Reconcile freeze state after an undo/redo restore drove the project
    /// back to `tracks` — the snapshot's `ProjectTrack.freeze`, read through
    /// [`FreezeStatus::from_persisted`] (ARCH-01 A-4). The undo restore
    /// calls it with the live statuses still in place, so it can see which
    /// caches the restore retires. The rendered cache is not part of undo
    /// history, so:
    ///
    /// - a track that was frozen but is not in the target (undo of a
    ///   freeze) has its cache detached from the engine and deleted from
    ///   disk;
    /// - a track the target has frozen (`Frozen` or `Stale`) whose engine
    ///   source is not attached — each one that was not frozen before the
    ///   restore, or that the restore re-added (`fresh`) — has its cache
    ///   decoded and re-attached
    ///   (`SetTrackFrozenSource`), as a disk load does; an undecodable or
    ///   missing cache leaves it `Stale` with nothing attached (FU-A4a);
    /// - a track that stays frozen across the restore keeps the
    ///   source the engine already holds (no re-decode); restored `Frozen`,
    ///   it is downgraded to `Stale` if its cache file is gone.
    ///
    /// The UPD-05 content baselines are left alone: a track restored
    /// `Frozen` must still go stale on its next content edit (FU-H2b).
    /// Any in-flight batch is abandoned — a restore stops the engine.
    ///
    /// `project_path` is the live project's `.rproj` path, whose sibling
    /// freeze directory holds the caches.
    /// `fresh`: tracks the restore has just added to the engine (ARCH-01
    /// A-13i), which hold no frozen source whatever their live status says
    /// — each has its cache attached as a disk load does.
    pub(crate) fn apply_freeze_restore(
        &mut self,
        tracks: &[crate::project::ProjectTrack],
        project_path: Option<&Path>,
        fresh: &std::collections::HashSet<TrackId>,
    ) {
        let target = tracks
            .iter()
            .map(|t| (t.id, FreezeStatus::from_persisted(&t.freeze)))
            .filter(|(_, status)| *status != FreezeStatus::Idle)
            .collect();
        self.reconcile_freeze_statuses(target, project_path, fresh);
    }

    /// The body of [`Self::apply_freeze_restore`], on a target already in
    /// live form.
    pub(crate) fn reconcile_freeze_statuses(
        &mut self,
        target: std::collections::HashMap<TrackId, FreezeStatus>,
        project_path: Option<&Path>,
        fresh: &std::collections::HashSet<TrackId>,
    ) {
        let dir = project_path.map(freeze_cache_dir_for);
        // The tracks whose engine source is attached right now: a live
        // `Frozen` / `Stale` status plays its cache (the engine attaches on
        // `FreezeCompleted`, a load or restore on decode), unless the
        // restore has just re-added the track without one.
        let attached: std::collections::HashSet<TrackId> = self
            .freeze
            .statuses
            .iter()
            .filter(|(id, status)| status.is_frozen() && !fresh.contains(id))
            .map(|(id, _)| *id)
            .collect();
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
            detach_and_delete_cache_in(self, id, dir.as_deref());
        }

        // Apply the target: attach the cache of every restored freeze the
        // engine does not hold, and downgrade any restored `Frozen` whose
        // cache is gone (or undecodable) to `Stale`. Track order, so the
        // engine sees the attaches deterministically.
        let mut target: Vec<(TrackId, FreezeStatus)> = target.into_iter().collect();
        target.sort_by_key(|(id, _)| *id);
        let mut reconciled = std::collections::HashMap::new();
        for (id, status) in target {
            let resolved = match status {
                status if status.is_frozen() && !attached.contains(&id) => {
                    self.attach_freeze_cache(id, status, dir.as_deref()).0
                }
                FreezeStatus::Frozen { mut cache_ref } => {
                    let exists = dir
                        .as_ref()
                        .is_some_and(|d| d.join(&cache_ref.cache_filename).exists());
                    if !exists {
                        cache_ref.status = FreezeCacheStatus::Stale;
                        FreezeStatus::Stale { cache_ref }
                    } else {
                        FreezeStatus::Frozen { cache_ref }
                    }
                }
                other => other,
            };
            reconciled.insert(id, resolved);
        }
        self.freeze.statuses = reconciled;
        self.freeze.queue = None;
    }
}

// ---------------------------------------------------------------------
// Read-only gating + stale invalidation (ba todo #576)
// ---------------------------------------------------------------------
//
// A frozen track's *inputs* — notes, lyrics, plugin params, instrument
// selection — are read-only while the cache stands in for the live chain.
// The pre-dispatch classifier in `update/gates.rs` spots an edit aimed at
// those inputs and, when the track is frozen, routes it here instead of
// dispatching it: the edit never mutates state or enters the undo stack,
// it just flips the freeze to `Stale` so the UI can offer a refreeze
// (the freeze banner offers it). Mixer controls (volume / pan / mute /
// solo / routing / sends) are *not* freeze inputs, so they never reach
// this path and stay fully live.
//
// Two checks catch what the gate cannot refuse:
//
// - Arrangement content (UPD-05): compose regeneration, bar shifts, tempo
//   and meter edits, engine echoes. `freeze_content_fingerprint` is
//   baselined when a cache becomes valid and compared after every
//   dispatch (`revalidate_frozen_content`).
// - Params the plugin moved itself (W4): an edit announced from its own
//   editor, or a values rescan while that editor is open (a preset from
//   its preset bar). `freeze_param_fingerprint` is compared around just
//   that update (`watch_frozen_params` / `settle_frozen_params`); it has
//   no standing baseline, because params arrive asynchronously after a
//   load.
//
// None of these compare against the engine's
// `FreezeCacheRef::render_fingerprint`: the engine hashes its own view
// (instance ids, resolved lanes), which the app cannot reproduce.

impl Resonance {
    /// Invalidate a frozen track to [`FreezeStatus::Stale`], keeping the
    /// (now-outdated) cache attached so playback still works until the user
    /// refreezes. Returns `true` when a `Frozen → Stale` transition
    /// happened; `Stale` stays stale (idempotent) and a track that isn't
    /// frozen is left untouched.
    pub(crate) fn invalidate_frozen_track(&mut self, track_id: TrackId) -> bool {
        match self.freeze.status(track_id) {
            FreezeStatus::Frozen { cache_ref } => {
                let mut cache_ref = cache_ref;
                cache_ref.status = FreezeCacheStatus::Stale;
                self.freeze
                    .set(track_id, FreezeStatus::Stale { cache_ref });
                true
            }
            // Already stale, or simply not frozen — nothing to invalidate.
            _ => false,
        }
    }

    /// Fingerprint of the plugin side of a track's frozen inputs: the
    /// chain (plugin ids in slot order, so a swap or reorder counts), each
    /// slot's bypass, every writable param value, and the FX-bypass flag.
    /// Read-only outputs (a load progress, a meter) move on their own and
    /// are left out. `None` when the track is gone.
    ///
    /// Not part of [`Self::freeze_content_fingerprint`]: params arrive
    /// asynchronously after a load, so a standing baseline would read
    /// their arrival as an edit. It is compared instead around the mirror
    /// updates that can move a frozen track's params without passing the
    /// pre-dispatch gate — the plugin changing them itself
    /// ([`Self::watch_frozen_params`]).
    pub(crate) fn freeze_param_fingerprint(&self, track_id: TrackId) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let track = self.registry.tracks.iter().find(|t| t.id == track_id)?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for slot in &track.plugins {
            slot.clap_plugin_id.hash(&mut h);
            slot.bypassed.hash(&mut h);
            for p in slot.params.iter().filter(|p| !p.read_only) {
                p.id.hash(&mut h);
                p.current_value.to_bits().hash(&mut h);
            }
        }
        track.fx_bypassed.hash(&mut h);
        Some(h.finish())
    }

    /// Before a mirror update that bypasses the pre-dispatch gate — a
    /// param the plugin changed itself (`ParamEditedByPlugin`), or a values
    /// rescan while its editor is open (a preset loaded from the plugin's
    /// own preset bar) — note the plugin's track and its param fingerprint
    /// if that track is `Frozen`. Hand the result to
    /// [`Self::settle_frozen_params`] after the update. `None` (nothing to
    /// check) for a plugin on a bus or the master, or on a track that isn't
    /// frozen, so the common case costs one map lookup.
    pub(crate) fn watch_frozen_params(
        &self,
        instance_id: resonance_audio::types::PluginInstanceId,
    ) -> Option<(TrackId, u64)> {
        let track_id = self.track_of_plugin(instance_id)?;
        if !matches!(self.freeze.status(track_id), FreezeStatus::Frozen { .. }) {
            return None;
        }
        Some((track_id, self.freeze_param_fingerprint(track_id)?))
    }

    /// After the update [`Self::watch_frozen_params`] watched: when the
    /// track's params moved, the cache no longer holds what the chain
    /// plays, so mark it `Stale`. A no-op rescan, or an edit to the value
    /// the param already had, leaves it `Frozen`. Returns whether it went
    /// stale.
    pub(crate) fn settle_frozen_params(&mut self, watch: Option<(TrackId, u64)>) -> bool {
        let Some((track_id, before)) = watch else {
            return false;
        };
        if self
            .freeze_param_fingerprint(track_id)
            .is_some_and(|after| after != before)
        {
            self.invalidate_frozen_track(track_id)
        } else {
            false
        }
    }

    /// Fingerprint of what a frozen track's cache was rendered *from* on
    /// the arrangement side: every MIDI clip on the track (position,
    /// length, trims, notes, lyrics in their file form — in position
    /// order, ids excluded so a
    /// re-derived clip with the same content matches) plus the tempo and
    /// meter maps, which move ticks in time, and the plugin-param
    /// automation lanes on the track's chain. Plugin params are left out:
    /// they arrive asynchronously after a load and are already covered by
    /// the pre-dispatch freeze gate. `None` when the track is gone
    /// (code review UPD-05).
    pub(crate) fn freeze_content_fingerprint(&self, track_id: TrackId) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        if !self.registry.tracks.iter().any(|t| t.id == track_id) {
            return None;
        }
        let mut clips: Vec<&MidiClipState> = self
            .midi_clips
            .iter()
            .filter(|c| c.track_id == track_id)
            .collect();
        clips.sort_by_key(|c| (c.start_sample, c.id));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for clip in clips {
            clip.start_sample.hash(&mut h);
            clip.duration_ticks.hash(&mut h);
            clip.trim_start_ticks.hash(&mut h);
            clip.trim_end_ticks.hash(&mut h);
            clip.notes.len().hash(&mut h);
            for n in clip.notes.iter() {
                n.note.hash(&mut h);
                n.velocity.to_bits().hash(&mut h);
                n.start_tick.hash(&mut h);
                n.duration_ticks.hash(&mut h);
            }
            // The lyrics' file form, so a restore that only normalises
            // the side-table's padding (A-2) is not a content change.
            self.compose
                .vocal_audio
                .file_lyrics(clip.id, clip.notes.len())
                .hash(&mut h);
        }
        format!("{:?}{:?}", self.tempo_events, self.signature_events).hash(&mut h);
        // The enabled plugin-param automation lanes on the track's chain,
        // which the freeze bakes into the cache (code review ENG-08).
        // Keyed by slot position, not instance id, so a reload that
        // re-issues ids reads as the same content.
        if let Some(track) = self.registry.tracks.iter().find(|t| t.id == track_id) {
            for (slot_index, slot) in track.plugins.iter().enumerate() {
                let mut lanes: Vec<_> = self
                    .automation
                    .lanes
                    .values()
                    .filter(|l| l.enabled)
                    .filter_map(|l| match l.target {
                        resonance_common::AutomationTarget::PluginParam { instance, param_id }
                            if instance == slot.instance_id =>
                        {
                            Some((param_id, l))
                        }
                        _ => None,
                    })
                    .collect();
                lanes.sort_by_key(|(param_id, _)| *param_id);
                for (param_id, lane) in lanes {
                    slot_index.hash(&mut h);
                    param_id.hash(&mut h);
                    for p in &lane.points {
                        p.time_frames.hash(&mut h);
                        p.value.to_bits().hash(&mut h);
                        p.curve.hash(&mut h);
                    }
                }
            }
        }
        Some(h.finish())
    }

    /// Remember a track's content fingerprint as the one its (valid)
    /// cache was rendered from. Called when a cache becomes valid: a
    /// freeze completing, a frozen cache re-attached on load.
    pub(crate) fn note_freeze_content_baseline(&mut self, track_id: TrackId) {
        if let Some(fp) = self.freeze_content_fingerprint(track_id) {
            self.freeze.content_baselines.insert(track_id, fp);
        }
    }

    /// After a dispatch: downgrade every `Frozen` track whose arrangement
    /// content drifted from its baseline to `Stale` (code review UPD-05).
    /// A track with a derived clip whose engine echo is still pending
    /// (a GUI regeneration tears the old clip down now and mirrors the
    /// new one on `MidiClipCreated`) is skipped until the echo lands, so a
    /// re-derive that reproduces the same notes doesn't read as a change.
    pub(crate) fn revalidate_frozen_content(&mut self) {
        if !self
            .freeze
            .statuses
            .values()
            .any(|s| matches!(s, FreezeStatus::Frozen { .. }))
        {
            return;
        }
        let frozen: Vec<(TrackId, u64)> = self
            .freeze
            .statuses
            .iter()
            .filter(|(_, s)| matches!(s, FreezeStatus::Frozen { .. }))
            .filter_map(|(id, _)| {
                self.freeze
                    .content_baselines
                    .get(id)
                    .map(|baseline| (*id, *baseline))
            })
            .collect();
        for (track_id, baseline) in frozen {
            let echo_pending = self
                .compose
                .derived_clips
                .iter()
                .any(|(&(_, _, t), clip_id)| {
                    t == track_id && !self.midi_clips.iter().any(|c| c.id == *clip_id)
                });
            if echo_pending {
                continue;
            }
            if self
                .freeze_content_fingerprint(track_id)
                .is_some_and(|fp| fp != baseline)
            {
                self.invalidate_frozen_track(track_id);
            }
        }
    }
}
