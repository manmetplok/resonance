//! GUI-side take-group state for loop/cycle recording & comping
//! (epic #15, design doc #165), mirrored from the engine.
//!
//! The app holds one [`TakeGroup`] per cycle-record run, reconstructed
//! *purely* from engine events — `TakeCaptured` appends a take (creating
//! the group on the first pass), while `TakeCompChanged` /
//! `ActiveTakeChanged` echo the comp / active-take the engine now plays.
//! There are no read-getters back into the engine, so this mirror is the
//! app's single source of truth for what the take lanes show. It mirrors
//! the [`AuxSendState`](super::AuxSendState) projection pattern.
//!
//! What a mirrored group *plays* is not decided here. Since todo #1395 the
//! resolution — active take → comp segment → latest take — is
//! [`resonance_common::effective_cover`], shared with the mixer, and comp
//! editing materializes it through
//! [`TakeGroup::effective_comp`](resonance_common::TakeGroup::effective_comp).
//! This module previously carried its own `fallback_take` /
//! `effective_segments` pair that mirrored the engine by assertion; both are
//! gone.

use std::collections::{HashMap, HashSet};

use resonance_audio::types::TrackId;
use resonance_common::{
    ClipId, CompSegment, Take, TakeContent, TakeGroup, TakeGroupId, TakeId, TimelineRange,
};

/// GUI-side mirror of the engine's take groups.
#[derive(Debug, Default)]
pub struct TakeGroupState {
    /// Every live take group. Insertion-ordered: a group is appended on
    /// its first captured pass and thereafter found by the engine's stable
    /// `group_id`.
    pub groups: Vec<TakeGroup>,
    /// `(group, take)` pairs whose recorded WAV was absent when the
    /// project loaded (todo #412).
    ///
    /// Only audio takes can be flagged — a MIDI take carries its notes
    /// inline and cannot go missing. The take itself stays in its group
    /// and the comp keeps referencing it, exactly as a
    /// [`PoolAsset`](super::pool::PoolAsset) missing its backing file
    /// stays in the pool: dropping it would silently punch a hole in the
    /// comp cover and make a later relink impossible. Held beside the
    /// groups rather than on `resonance_common::Take` because it is a fact
    /// about *this machine's filesystem right now*, not about the project
    /// — it must never be written back to disk.
    pub missing_takes: HashSet<(TakeGroupId, TakeId)>,
    /// Waveform peaks for each audio take, derived from its recording by
    /// [`load_take_peaks`](crate::project::load_take_peaks) at the moment
    /// the app learned of the take — a `TakeCaptured` echo, or a project
    /// load (ba todo #1400).
    ///
    /// Keyed like [`missing_takes`](Self::missing_takes) rather than by
    /// `clip_ref`, so the two travel together: every site that forgets a
    /// take forgets both, with one key and no clip lookup. The `clip_ref`
    /// the table was read from is stored **beside** it and checked on
    /// every hit — see [`TakePeaks`].
    ///
    /// Beside the groups, and for the same reason the missing set is: a
    /// peak table is a fact about a file on *this* machine, cheap to
    /// re-derive and never written back to disk. Absent for a MIDI take
    /// (its notes are its content) and for an audio take whose recording
    /// could not be read.
    ///
    /// **This is a cache, and the one piece of this mirror that outlives a
    /// snapshot rebuild** — see [`clear_for_snapshot`](Self::clear_for_snapshot).
    ///
    /// **Private, unlike its neighbours.** Every read has to prove which
    /// recording it is asking about, and a `pub` field would let a caller
    /// take the table without proving anything —
    /// [`peaks`](Self::peaks) and [`has_peaks`](Self::has_peaks) are the
    /// only ways in, and both compare the `clip_ref`.
    peaks: HashMap<(TakeGroupId, TakeId), TakePeaks>,
}

/// A take's cached waveform, and the recording it was read from.
///
/// The `clip_ref` is what makes the cache safe to carry across a snapshot
/// rebuild. `(group, take)` names a *slot in the mirror*, not a recording:
/// a capture that was undone and re-recorded lands a different pass under
/// the same pair, and diff replay can then restore the earlier snapshot
/// with no capture echo to refresh the table. Every read compares the ref,
/// so a mismatch is a miss and is re-read — the failure mode designed out
/// here is a lane confidently drawing a waveform that is not the take's.
#[derive(Debug, Clone)]
pub struct TakePeaks {
    /// The `clip_ref` of the recording [`peaks`](Self::peaks) came from.
    pub clip_ref: ClipId,
    /// One `(min, max)` pair per `WAVEFORM_PEAK_FRAMES` frames.
    pub peaks: Vec<(f32, f32)>,
}

impl TakeGroupState {
    /// Drop every mirrored group (and its missing-file flags and peaks).
    ///
    /// Called when a project load wipes the previous project's runtime
    /// registry: `ClearAll` empties the engine's take-group map without
    /// echoing a per-group removal, so the mirror has to be emptied
    /// explicitly or project B inherits project A's groups — and with
    /// them `clip_ref`s into a different project's `audio/` directory.
    ///
    /// Peaks go too, and must: clip ids are per-project, so project B's
    /// `clip_ref` 100 names a different WAV from project A's.
    pub fn clear(&mut self) {
        self.groups.clear();
        self.missing_takes.clear();
        self.peaks.clear();
    }

    /// Empty the mirror for a **rebuild from a snapshot of this same
    /// session** — the undo/redo diff-replay path — keeping the peak
    /// cache (ba todo #1400).
    ///
    /// Undo and redo rebuild the take lanes from a `ProjectFile` on every
    /// history step, *including steps that touch no take at all*: a fader
    /// undo runs this too. Dropping the peaks there would make each step
    /// re-mmap and re-scan every take WAV in the project — measured at
    /// ~3.2 ms per recorded minute, so ~100 ms per step for a session
    /// holding half an hour of takes, on a hold-to-repeat gesture. There
    /// is nothing to re-read: a recording never changes, and a table that
    /// does not match the take it is filed under is rejected on read
    /// ([`TakePeaks`]).
    ///
    /// The missing flags are **not** kept, deliberately. They are a claim
    /// about the filesystem right now, and re-deriving one costs a failed
    /// `open` rather than a scan — so a take whose WAV was deleted
    /// mid-session is still re-flagged on the next history step rather
    /// than going quiet, which is what the old `exists()` probe here was
    /// for.
    pub fn clear_for_snapshot(&mut self) {
        self.groups.clear();
        self.missing_takes.clear();
    }

    /// File the peaks read from `clip_ref`'s recording for this take.
    ///
    /// Replaces any previous table for the key, so a re-delivered capture
    /// refreshes rather than duplicating.
    pub fn set_peaks(
        &mut self,
        group_id: TakeGroupId,
        take_id: TakeId,
        clip_ref: ClipId,
        peaks: Vec<(f32, f32)>,
    ) {
        self.peaks
            .insert((group_id, take_id), TakePeaks { clip_ref, peaks });
    }

    /// This take's waveform peaks, or an empty slice when it has none —
    /// a MIDI take, a recording that could not be read, a capture the app
    /// could not resolve a project directory for, or a cached table
    /// belonging to a *different* recording under the same key. Borrows,
    /// so the draw pass never clones a peak table per frame.
    pub fn peaks(
        &self,
        group_id: TakeGroupId,
        take_id: TakeId,
        clip_ref: ClipId,
    ) -> &[(f32, f32)] {
        match self.peaks.get(&(group_id, take_id)) {
            Some(cached) if cached.clip_ref == clip_ref => &cached.peaks,
            _ => &[],
        }
    }

    /// Whether this take's recording has already been read into the cache
    /// — the check that lets a snapshot rebuild skip the read entirely.
    pub fn has_peaks(&self, group_id: TakeGroupId, take_id: TakeId, clip_ref: ClipId) -> bool {
        self.peaks
            .get(&(group_id, take_id))
            .is_some_and(|cached| cached.clip_ref == clip_ref)
    }

    /// Drop this take's cached waveform — its recording became
    /// unreadable, so the table is a memory of a file that is no longer
    /// there and must not survive as a cache hit.
    pub fn forget_peaks(&mut self, group_id: TakeGroupId, take_id: TakeId) {
        self.peaks.remove(&(group_id, take_id));
    }

    /// Flag `take_id` in `group_id` as having no recorded audio on disk.
    pub fn mark_missing(&mut self, group_id: TakeGroupId, take_id: TakeId) {
        self.missing_takes.insert((group_id, take_id));
    }

    /// True when this take's recorded audio was absent at load time.
    pub fn is_missing(&self, group_id: TakeGroupId, take_id: TakeId) -> bool {
        self.missing_takes.contains(&(group_id, take_id))
    }

    /// True when any mirrored take is missing its recorded audio.
    pub fn has_missing(&self) -> bool {
        !self.missing_takes.is_empty()
    }

    /// The group carrying `group_id`, if mirrored.
    pub fn group(&self, group_id: TakeGroupId) -> Option<&TakeGroup> {
        self.groups.iter().find(|g| g.id == group_id)
    }

    /// Mutable access to the group carrying `group_id`, if mirrored.
    pub fn group_mut(&mut self, group_id: TakeGroupId) -> Option<&mut TakeGroup> {
        self.groups.iter_mut().find(|g| g.id == group_id)
    }

    /// Mirror a `TakeCaptured` event: append the finished loop pass as a
    /// take, creating the group on the first pass.
    ///
    /// `group_id` is the engine's stable key for a **track + loop slot**
    /// (doc #165) — one lane per slot, for as many record runs as the user
    /// presses over it (todo #1392), so a second pass *and* a second run
    /// both fold into the same group rather than starting a new one. A
    /// take whose id already exists in the group replaces it, keeping the
    /// mirror idempotent if an event is re-delivered.
    ///
    /// A take's `pass_index` is likewise group-relative, not run-relative:
    /// it is the take's ordinal within its lane, which is what makes the
    /// lane sort and label correctly (`T1`, `T2`, …) across a stop and a
    /// reload.
    ///
    /// The caller hands over a fully-formed [`Take`] — including the
    /// `extent` the engine reported and the wall-clock stamp the event
    /// omits — rather than a widening list of scalars.
    pub fn take_captured(
        &mut self,
        group_id: TakeGroupId,
        track_id: TrackId,
        slot: TimelineRange,
        take: Take,
    ) {
        // A freshly captured take has its WAV on disk by definition, so
        // clear any missing-file flag a prior load left on this slot.
        // Its peaks are filed by the caller, which is the only place that
        // can read them; drop a stale table here so a re-used id can
        // never draw the previous take's waveform if that read fails.
        let take_id = take.id;
        self.missing_takes.remove(&(group_id, take_id));
        self.peaks.remove(&(group_id, take_id));
        match self.group_mut(group_id) {
            Some(group) => match group.takes.iter_mut().find(|t| t.id == take_id) {
                Some(existing) => *existing = take,
                None => group.add_take(take),
            },
            None => {
                let mut group = TakeGroup::new(group_id, track_id, slot);
                group.add_take(take);
                self.groups.push(group);
            }
        }
    }

    /// Mirror a `TakeCompChanged` event: adopt the comp the engine now
    /// plays/bounces, replacing the group's segments wholesale. Unknown
    /// groups are ignored (the capture that creates the group always
    /// precedes any comp change).
    ///
    /// This is the *only* way the mirror's comp is meant to move: the
    /// update handlers apply an edit through here and send the matching
    /// `SetTakeComp`, and the engine's `TakeCompChanged` echo re-applies
    /// the same segments idempotently (todo #411).
    pub fn comp_changed(&mut self, group_id: TakeGroupId, segments: Vec<CompSegment>) {
        if let Some(group) = self.group_mut(group_id) {
            group.comp.segments = segments;
        }
    }

    /// Mirror an `ActiveTakeChanged` event: set (or clear, with `None`) the
    /// take soloed for full-slot playback. Unknown groups are ignored.
    ///
    /// Pairs with [`comp_changed`](Self::comp_changed); both are routed
    /// from `engine_events::takes` (todo #411).
    pub fn active_take_changed(&mut self, group_id: TakeGroupId, take_id: Option<TakeId>) {
        if let Some(group) = self.group_mut(group_id) {
            group.active_take = take_id;
        }
    }

    /// Drop `take_id` from `group_id`, returning whether it was there.
    ///
    /// The take's missing-media flag and its peak table go with it, so a
    /// later group that happens to reuse the id inherits neither. Only the
    /// take itself is removed — the caller is responsible for pushing a
    /// comp that no longer references it, because a `CompSegment` naming a
    /// deleted take renders as a hole.
    pub fn remove_take(&mut self, group_id: TakeGroupId, take_id: TakeId) -> bool {
        self.missing_takes.remove(&(group_id, take_id));
        self.peaks.remove(&(group_id, take_id));
        let Some(group) = self.group_mut(group_id) else {
            return false;
        };
        let Some(idx) = group.takes.iter().position(|t| t.id == take_id) else {
            return false;
        };
        group.takes.remove(idx);
        if group.active_take == Some(take_id) {
            group.active_take = None;
        }
        true
    }

    /// Drop the whole lane `group_id`, returning whether it was there.
    ///
    /// Mirrors `AudioEvent::TakeGroupRemoved` (ba todo #1397). Every
    /// missing-media flag **and every cached waveform** the group carried
    /// goes with it, for the same reason [`Self::remove_take`] drops one
    /// take's: a later group reusing the id must not inherit either.
    ///
    /// The peak half was missed when #1397 and #1400 met — #1397 wrote
    /// this against a state that had no peak map, so it cited
    /// `remove_take`'s reasoning while doing half of it, and
    /// [`peaks`](Self::peaks)'s promise that "every site that forgets a
    /// take forgets both" was false at exactly this one site. It leaked
    /// rather than corrupted, because a lookup compares the `clip_ref`
    /// ([`TakePeaks`]) and a reused group id therefore missed instead of
    /// drawing the orphaned table — but the memory was held for the rest
    /// of the session, and a promise with one exception is not a promise.
    pub fn remove_group(&mut self, group_id: TakeGroupId) -> bool {
        let Some(idx) = self.groups.iter().position(|g| g.id == group_id) else {
            return false;
        };
        self.groups.remove(idx);
        self.missing_takes.retain(|(gid, _)| *gid != group_id);
        self.peaks.retain(|(gid, _), _| *gid != group_id);
        true
    }

    /// True when this group's active take mutes its recorded audio.
    ///
    /// Soloing a **MIDI** take resolves to zero audio spans while every
    /// audio take in the group stays governed (skipped on the ordinary
    /// clip path), so the lane goes silent on the audio path — by design
    /// (ba doc #292), but indistinguishable from a bug unless the UI says
    /// so. Exposed here rather than inferred in the view so the rule lives
    /// next to the mirror it is a fact about.
    pub fn active_take_silences_audio(&self, group_id: TakeGroupId) -> bool {
        let Some(group) = self.group(group_id) else {
            return false;
        };
        let Some(active) = group.active_take.and_then(|id| group.take(id)) else {
            return false;
        };
        matches!(active.content, TakeContent::Midi { .. })
            && group
                .takes
                .iter()
                .any(|t| matches!(t.content, TakeContent::Audio { .. }))
    }
}
