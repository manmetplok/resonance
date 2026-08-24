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

use std::collections::HashSet;

use resonance_audio::types::TrackId;
use resonance_common::{
    CompSegment, Take, TakeContent, TakeGroup, TakeGroupId, TakeId, TimelineRange,
};

/// The take a group plays where its comp names none.
///
/// Mirrors the engine's fallback in `mixer::take_comp::resolve_spans`
/// **exactly**: the last *audio* take in capture order, because that is
/// what is actually audible for a group whose comp is still empty. Only
/// when a group holds no audio at all (a MIDI-only instrument lane) does
/// it fall back to the last take of any kind, so a MIDI lane still has an
/// addressable cover to comp against.
///
/// Getting this wrong would make the first comp edit on a fresh group
/// *change what is heard* — [`effective_segments`] materializes the
/// engine's implicit cover before editing it, and that materialization is
/// only inaudible if it names the same take the engine had chosen.
pub fn fallback_take(group: &TakeGroup) -> Option<TakeId> {
    group
        .takes
        .iter()
        .rev()
        .find(|t| matches!(t.content, TakeContent::Audio { .. }))
        .or_else(|| group.takes.last())
        .map(|t| t.id)
}

/// The group's comp as an explicit, editable cover of its slot.
///
/// A group that has never been comped carries **no** segments, but it is
/// not silent: the engine covers the whole slot with [`fallback_take`].
/// Comp editing therefore starts by materializing that implicit cover, so
/// a first split or promote edits what the user is hearing instead of
/// appearing to do nothing (`Comp::split_comp` on an empty comp is a
/// no-op, and a promote over an empty comp would leave the rest of the
/// slot as a real hole).
///
/// Returns an empty vector only for a group with no takes at all, which
/// has nothing to cover the slot with.
pub fn effective_segments(group: &TakeGroup) -> Vec<CompSegment> {
    if !group.comp.segments.is_empty() {
        return group.comp.segments.clone();
    }
    match fallback_take(group) {
        Some(take_id) => vec![CompSegment {
            range: group.slot,
            take_id,
        }],
        None => Vec::new(),
    }
}

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
}

impl TakeGroupState {
    /// Drop every mirrored group (and its missing-file flags).
    ///
    /// Called when a project load wipes the previous project's runtime
    /// registry: `ClearAll` empties the engine's take-group map without
    /// echoing a per-group removal, so the mirror has to be emptied
    /// explicitly or project B inherits project A's groups — and with
    /// them `clip_ref`s into a different project's `audio/` directory.
    pub fn clear(&mut self) {
        self.groups.clear();
        self.missing_takes.clear();
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
    /// `group_id` is the engine's stable per-track-per-run key — it stands
    /// in for the track + loop `slot` the run records over (doc #165), so a
    /// second pass folds into the same group rather than starting a new
    /// one. A take whose id already exists in the group replaces it,
    /// keeping the mirror idempotent if an event is re-delivered.
    #[allow(clippy::too_many_arguments)]
    pub fn take_captured(
        &mut self,
        group_id: TakeGroupId,
        take_id: TakeId,
        track_id: TrackId,
        slot: TimelineRange,
        pass_index: u32,
        captured_at: i64,
        content: TakeContent,
    ) {
        // A freshly captured take has its WAV on disk by definition, so
        // clear any missing-file flag a prior load left on this slot.
        self.missing_takes.remove(&(group_id, take_id));
        let take = Take::new(take_id, pass_index, captured_at, content);
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
    /// The take's missing-media flag goes with it, so a later group that
    /// happens to reuse the id does not inherit a stale one. Only the take
    /// itself is removed — the caller is responsible for pushing a comp
    /// that no longer references it, because a `CompSegment` naming a
    /// deleted take renders as a hole.
    pub fn remove_take(&mut self, group_id: TakeGroupId, take_id: TakeId) -> bool {
        self.missing_takes.remove(&(group_id, take_id));
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
