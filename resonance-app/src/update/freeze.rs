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
use std::path::{Path, PathBuf};

use iced::Task;
use resonance_audio::types::{AudioCommand, TrackId, TrackType};
use resonance_common::{
    compute_fingerprint, FreezeCacheStatus, FreezeFingerprintBuilder, TrackFreezeState,
};

use crate::message::{FreezeMessage, Message};
use crate::state::{FreezeQueue, FreezeStatus, MidiClipState, TrackState};
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: FreezeMessage) -> Task<Message> {
    // Every freeze action is reachable from the track context menu (ba
    // todo #581); acting on an entry closes the menu, and the messages
    // arriving from other surfaces (header toggle, shortcuts) are no-ops
    // on an already-`None` menu.
    r.interaction.track_menu = None;
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
        r.error_message = Some("Save the project before revealing the freeze cache".into());
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        r.error_message = Some(format!("Could not open freeze cache directory: {e}"));
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
    // An offline control measurement holds the offline renderer
    // exclusively; a freeze render on top of it would drive the same
    // live plugin instances from two renderers at once (mirrors
    // `meter.measure` refusing while a freeze runs).
    if r.offline_measure_in_progress() {
        r.error_message = Some("A measurement is in progress; freeze again when it finishes".into());
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
        r.error_message = Some("Stop transport before freezing".into());
        return;
    }
    // Same offline-renderer exclusion as `freeze_one`.
    if r.offline_measure_in_progress() {
        r.error_message = Some("A measurement is in progress; freeze again when it finishes".into());
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
    r.interaction
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
            // Live / not-frozen tracks carry no cache to restore.
            let Some(cache_ref) = state.cache_ref.clone() else {
                continue;
            };
            if !state.is_frozen {
                continue;
            }
            let path = dir.join(&cache_ref.cache_filename);
            match resonance_audio::read_freeze_cache(&path, cache_ref.clone()) {
                Ok(source) => {
                    // Cache is present and decodable: replay it instead of
                    // re-rendering the live chain.
                    let _ = self.engine.send(AudioCommand::SetTrackFrozenSource {
                        track_id,
                        source: Some(source),
                    });
                    // Mirror the persisted status. A project saved while a
                    // track was stale stays stale (so the UI keeps offering
                    // a refreeze) but still plays the cache it has.
                    let status = match cache_ref.status {
                        FreezeCacheStatus::Stale => FreezeStatus::Stale { cache_ref },
                        _ => FreezeStatus::Frozen { cache_ref },
                    };
                    self.freeze.set(track_id, status);
                    // The project was saved with this cache valid, so the
                    // content just replayed is what it was rendered from.
                    self.note_freeze_content_baseline(track_id);
                }
                Err(e) => {
                    // Missing / corrupt cache: load stale and offer a
                    // refreeze rather than failing the whole project.
                    eprintln!(
                        "Freeze cache for track {track_id} unavailable ({e}); loading as stale"
                    );
                    let mut cache_ref = cache_ref;
                    cache_ref.status = FreezeCacheStatus::Stale;
                    self.freeze
                        .set(track_id, FreezeStatus::Stale { cache_ref });
                }
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
// (banner is the UI todo). Mixer controls (volume / pan / mute / solo /
// routing / sends) are *not* freeze inputs, so they never reach this path
// and stay fully live.

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

    /// Recompute the resonance-common input fingerprint over a track's
    /// current frozen inputs (notes, lyrics, plugin params, instrument
    /// selection). Returns `None` when the track no longer exists. The
    /// fingerprint is order-stable: clips and plugins are folded in a fixed
    /// order so reshuffling the backing `Vec`s never changes the hash.
    pub(crate) fn compute_track_freeze_fingerprint(&self, track_id: TrackId) -> Option<u64> {
        let track = self.registry.tracks.iter().find(|t| t.id == track_id)?;

        // Notes + lyrics: every MIDI clip bound to this track, in clip-id
        // order so the backing Vec's order can't perturb the hash.
        let mut clips: Vec<&MidiClipState> = self
            .midi_clips
            .iter()
            .filter(|c| c.track_id == track_id)
            .collect();
        clips.sort_by_key(|c| c.id);

        let mut notes = Vec::new();
        let mut lyrics = Vec::new();
        for clip in clips {
            notes.extend_from_slice(&clip.id.to_le_bytes());
            notes.extend_from_slice(&clip.start_sample.to_le_bytes());
            notes.extend_from_slice(&clip.duration_ticks.to_le_bytes());
            notes.extend_from_slice(&clip.trim_start_ticks.to_le_bytes());
            notes.extend_from_slice(&clip.trim_end_ticks.to_le_bytes());
            for n in &clip.notes {
                notes.push(n.note);
                notes.extend_from_slice(&n.velocity.to_bits().to_le_bytes());
                notes.extend_from_slice(&n.start_tick.to_le_bytes());
                notes.extend_from_slice(&n.duration_ticks.to_le_bytes());
            }
            if let Some(clip_lyrics) = self.compose.vocal_audio.clip_lyrics.get(&clip.id) {
                for syllable in clip_lyrics {
                    lyrics.extend_from_slice(syllable.as_bytes());
                    lyrics.push(0); // NUL-separate syllables so "a","b" ≠ "ab"
                }
            }
        }

        // Plugin params + instrument selection: the whole chain in slot
        // order, plus the FX-bypass flag (it changes the post-FX render
        // that freeze captured).
        let mut plugin_params = Vec::new();
        for slot in &track.plugins {
            plugin_params.extend_from_slice(slot.clap_plugin_id.as_bytes());
            plugin_params.push(0);
            for p in &slot.params {
                plugin_params.extend_from_slice(&p.id.to_le_bytes());
                plugin_params.extend_from_slice(&p.current_value.to_bits().to_le_bytes());
            }
        }
        plugin_params.push(track.fx_bypassed as u8);

        // The instrument is the chain's first plugin (slot 0 on an
        // instrument/vocal track); empty when the track has no synth yet.
        let instrument_id = track
            .plugins
            .first()
            .map(|p| p.clap_plugin_id.clone())
            .unwrap_or_default();

        let inputs = FreezeFingerprintBuilder::new()
            .with_notes(notes)
            .with_lyrics(lyrics)
            .with_plugin_params(plugin_params)
            .with_instrument_id(instrument_id)
            .build();
        Some(compute_fingerprint(&inputs))
    }

    /// Whether a frozen track's current inputs no longer match the
    /// fingerprint captured when its cache was rendered. `false` when the
    /// track isn't frozen (no cache to compare against) or the recompute
    /// fails.
    pub(crate) fn freeze_inputs_changed(&self, track_id: TrackId) -> bool {
        let Some(cache_ref) = self.freeze.status(track_id).cache_ref().cloned() else {
            return false;
        };
        match self.compute_track_freeze_fingerprint(track_id) {
            Some(fingerprint) => fingerprint != cache_ref.render_fingerprint,
            None => false,
        }
    }

    /// Fingerprint of what a frozen track's cache was rendered *from* on
    /// the arrangement side: every MIDI clip on the track (position,
    /// length, trims, notes, lyrics — in position order, ids excluded so a
    /// re-derived clip with the same content matches) plus the tempo and
    /// meter maps, which move ticks in time. Plugin params are left out:
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
            for n in &clip.notes {
                n.note.hash(&mut h);
                n.velocity.to_bits().hash(&mut h);
                n.start_tick.hash(&mut h);
                n.duration_ticks.hash(&mut h);
            }
            self.compose
                .vocal_audio
                .clip_lyrics
                .get(&clip.id)
                .hash(&mut h);
        }
        format!("{:?}{:?}", self.tempo_events, self.signature_events).hash(&mut h);
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

    /// Recompute the input fingerprint to *confirm* staleness on a suspected
    /// change: if a still-`Frozen` track's inputs have drifted from the
    /// rendered cache, downgrade it to `Stale`. Returns `true` when it
    /// transitioned. Unlike [`invalidate_frozen_track`] (which trusts the
    /// caller that a change happened), this verifies via the fingerprint, so
    /// it's safe to call on changes that may turn out to be no-ops.
    pub(crate) fn revalidate_frozen_track(&mut self, track_id: TrackId) -> bool {
        if matches!(self.freeze.status(track_id), FreezeStatus::Frozen { .. })
            && self.freeze_inputs_changed(track_id)
        {
            self.invalidate_frozen_track(track_id)
        } else {
            false
        }
    }
}
