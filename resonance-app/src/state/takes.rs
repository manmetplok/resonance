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

use resonance_audio::types::TrackId;
use resonance_common::{
    CompSegment, Take, TakeContent, TakeGroup, TakeGroupId, TakeId, TimelineRange,
};

/// GUI-side mirror of the engine's take groups.
#[derive(Debug, Default)]
pub struct TakeGroupState {
    /// Every live take group. Insertion-ordered: a group is appended on
    /// its first captured pass and thereafter found by the engine's stable
    /// `group_id`.
    pub groups: Vec<TakeGroup>,
}

impl TakeGroupState {
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
    /// The engine-side echo is todo #409's; until that variant exists on
    /// `AudioEvent` nothing in `engine_events` calls this, but the
    /// projection is the app's and is covered directly by its tests.
    pub fn comp_changed(&mut self, group_id: TakeGroupId, segments: Vec<CompSegment>) {
        if let Some(group) = self.group_mut(group_id) {
            group.comp.segments = segments;
        }
    }

    /// Mirror an `ActiveTakeChanged` event: set (or clear, with `None`) the
    /// take soloed for full-slot playback. Unknown groups are ignored.
    ///
    /// Pairs with [`comp_changed`](Self::comp_changed) and waits on the
    /// same todo #409 event variant.
    pub fn active_take_changed(&mut self, group_id: TakeGroupId, take_id: Option<TakeId>) {
        if let Some(group) = self.group_mut(group_id) {
            group.active_take = take_id;
        }
    }
}
