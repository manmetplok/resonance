//! Update handlers for take-lane comping (design doc #165, epic #15,
//! todo #411).
//!
//! Each [`TakeMessage`] resolves to at most one `SetTakeComp` plus one
//! `SetActiveTake`. The app-side mirror
//! ([`TakeGroupState`](crate::state::TakeGroupState)) is updated
//! optimistically so the view reflects the edit this frame, and the
//! engine's `TakeCompChanged` / `ActiveTakeChanged` echoes re-apply the
//! same values idempotently through `engine_events::takes` — the same
//! optimistic-then-confirmed shape `update::automation` uses.
//!
//! # One planner, two callers
//!
//! [`plan`] is a pure function from `(&Resonance, &TakeMessage)` to the
//! edit that message performs, or `None` when it would change nothing.
//! It is called twice per message: once by
//! [`Resonance::take_edit_is_refused`](crate::Resonance::take_edit_is_refused)
//! as a pre-dispatch gate, and once by [`handle`] to apply the result.
//! Gating on the *same* predicate that applies the edit is what keeps a
//! refused edit from spending an undo entry and bumping the control-API
//! revision — `update_inner` does both before dispatch, so a handler that
//! merely returned early would leave a phantom edit in the history (the
//! rule ba todo #1261 established for a refused chain reorder).
//!
//! # Three rules the engine does not enforce
//!
//! 1. **Segments are derived from the slot, never trusted from the UI.**
//!    `AudioCommand::SetTakeComp` performs no validation whatsoever: it
//!    stores what it is given and renders it. Every comp this module
//!    sends is built by [`effective_segments`] + the `resonance_common`
//!    comp helpers, which keep the cover sorted, non-overlapping and
//!    inside the slot.
//! 2. **A promote never offers a region its take cannot fill.** A take's
//!    audible extent is its recorded clip's, not the group's slot: a pass
//!    that punched in late, or was cut short at stop, covers less. Since
//!    todo #409 the renderer clamps such a span to the clip and ramps at
//!    its real edge rather than panicking — but the uncovered part of the
//!    segment is still a silent hole in the composite, so
//!    [`audible_extent`] clamps it away up front.
//! 3. **A rejected selection is never mirrored.** `SetActiveTake` with a
//!    take the group does not hold is dropped by the engine with no echo,
//!    so the app validates before sending instead of assuming success.
//!
//! # Two limits, both engine-side
//!
//! * Deleting a group's **last** take is refused. There is no
//!   take-removal command, and a group whose comp is empty falls back to
//!   its most recent pass, so the engine would keep playing what the user
//!   deleted. Lifting this needs an engine-side removal command.
//! * A restore (undo/redo) re-asserts every mirrored group's comp and
//!   active take onto the engine, but cannot *create* a group the engine
//!   has never captured — see `undo::snapshot::resync_take_comps` and
//!   todo #1394.

use iced::Task;
use resonance_audio::types::AudioCommand;
use resonance_common::{
    Comp, CompSegment, TakeContent, TakeGroup, TakeGroupId, TakeId, TimelineRange,
};

use crate::message::{Message, TakeMessage};
use crate::state::takes::effective_segments;
use crate::Resonance;

/// The mutation a [`TakeMessage`] performs, resolved against current
/// state. `None` fields are "leave alone / send nothing".
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TakeEdit {
    pub group_id: TakeGroupId,
    /// The comp cover to adopt and push, when the edit changes it.
    pub comp: Option<Vec<CompSegment>>,
    /// The active take to adopt and push, when the edit changes it.
    /// `Some(None)` clears the solo.
    pub active_take: Option<Option<TakeId>>,
    /// A take to drop from the mirror before the comp is applied.
    pub delete_take: Option<TakeId>,
}

pub fn handle(r: &mut Resonance, m: TakeMessage) -> Task<Message> {
    // A message whose plan is empty was already refused by the gate; the
    // second check costs one lookup and keeps `handle` correct on its own
    // (the control API and tests can call `dispatch` directly).
    if let Some(edit) = plan(r, &m) {
        apply(r, edit);
    }
    Task::none()
}

/// Resolve `m` against current state, or `None` when it would change
/// nothing. Pure: no mutation, no engine traffic.
pub(crate) fn plan(r: &Resonance, m: &TakeMessage) -> Option<TakeEdit> {
    match m {
        TakeMessage::SetActiveTake { group_id, take_id } => plan_active(r, *group_id, *take_id),
        TakeMessage::SplitCompAtPlayhead { group_id } => plan_split(r, *group_id),
        TakeMessage::PromoteTakeSegment {
            group_id,
            take_id,
            range,
        } => plan_promote(r, *group_id, *take_id, *range),
        TakeMessage::DeleteTake { group_id, take_id } => plan_delete(r, *group_id, *take_id),
    }
}

/// Apply a planned edit: mirror first (so the view is right this frame),
/// then push the engine commands that make it audible.
fn apply(r: &mut Resonance, edit: TakeEdit) {
    let TakeEdit {
        group_id,
        comp,
        active_take,
        delete_take,
    } = edit;

    if let Some(take_id) = delete_take {
        r.take_groups.remove_take(group_id, take_id);
    }
    if let Some(segments) = comp {
        r.take_groups.comp_changed(group_id, segments.clone());
        let _ = r
            .engine
            .send(AudioCommand::SetTakeComp { group_id, segments });
    }
    if let Some(take_id) = active_take {
        r.take_groups.active_take_changed(group_id, take_id);
        let _ = r
            .engine
            .send(AudioCommand::SetActiveTake { group_id, take_id });
    }
}

// ---------------------------------------------------------------------------
// Per-message planning
// ---------------------------------------------------------------------------

fn plan_active(r: &Resonance, group_id: TakeGroupId, take_id: Option<TakeId>) -> Option<TakeEdit> {
    let group = r.take_groups.group(group_id)?;
    // A take the group does not hold is refused rather than sent: the
    // engine drops it silently and echoes nothing (ba doc #292), so
    // sending it would desync the mirror from what is played.
    if let Some(id) = take_id {
        group.take(id)?;
    }
    if group.active_take == take_id {
        return None;
    }
    Some(TakeEdit {
        group_id,
        comp: None,
        active_take: Some(take_id),
        delete_take: None,
    })
}

fn plan_split(r: &Resonance, group_id: TakeGroupId) -> Option<TakeEdit> {
    let group = r.take_groups.group(group_id)?;
    let pos = r.transport.playhead;
    // Strictly inside: a cut at either slot edge produces no boundary the
    // comp did not already have.
    if pos <= group.slot.start || pos >= group.slot.end() {
        return None;
    }
    let mut comp = Comp {
        segments: effective_segments(group),
    };
    if comp.segments.is_empty() {
        return None; // nothing recorded yet — no cover to cut
    }
    comp.split_comp(pos);
    finish_comp_edit(group, comp.segments)
}

fn plan_promote(
    r: &Resonance,
    group_id: TakeGroupId,
    take_id: TakeId,
    range: TimelineRange,
) -> Option<TakeEdit> {
    let group = r.take_groups.group(group_id)?;
    let take = group.take(take_id)?;
    let target = intersect(range, audible_extent(r, group, take));
    if target.is_empty() {
        return None;
    }
    let mut comp = Comp {
        segments: effective_segments(group),
    };
    comp.promote(target, take_id);
    finish_comp_edit(group, comp.segments)
}

fn plan_delete(r: &Resonance, group_id: TakeGroupId, take_id: TakeId) -> Option<TakeEdit> {
    let group = r.take_groups.group(group_id)?;
    group.take(take_id)?;
    // The last take cannot go: with no takes left the comp is empty, and
    // an empty comp is exactly the state in which the engine falls back to
    // playing the most recent pass — the one just deleted. Refusing is the
    // honest outcome until an engine-side removal command exists.
    if group.takes.len() < 2 {
        return None;
    }
    let segments = cover_without(group, take_id);
    Some(TakeEdit {
        group_id,
        comp: Some(segments),
        // The solo goes with the take it named; otherwise it is untouched
        // (deleting some other take does not end a solo).
        active_take: (group.active_take == Some(take_id)).then_some(None),
        delete_take: Some(take_id),
    })
}

/// Package a rebuilt cover into an edit, dropping it when the comp is
/// unchanged, and clear any solo that would keep the edit inaudible.
///
/// An active take overrides the comp entirely (doc #165), so a split or
/// promote made while one is soloed would change nothing the user can
/// hear. Editing the comp is therefore taken as "play the comp" and ends
/// the solo — the same way selecting a comped region exits take-solo in
/// Logic and Pro Tools.
fn finish_comp_edit(group: &TakeGroup, segments: Vec<CompSegment>) -> Option<TakeEdit> {
    let comp_changed = segments != group.comp.segments;
    let clear_solo = group.active_take.is_some();
    if !comp_changed && !clear_solo {
        return None;
    }
    Some(TakeEdit {
        group_id: group.id,
        comp: comp_changed.then_some(segments),
        active_take: clear_solo.then_some(None),
        delete_take: None,
    })
}

/// The cover the slot should have once `deleted` is gone: its segments
/// removed, and the holes they leave handed to the take the group would
/// otherwise fall back to.
///
/// Leaving the holes would be worse than a wrong guess — a gap in the comp
/// renders as silence in the middle of the part, and the deleted take is
/// the one thing that cannot fill it.
fn cover_without(group: &TakeGroup, deleted: TakeId) -> Vec<CompSegment> {
    let kept: Vec<CompSegment> = effective_segments(group)
        .into_iter()
        .filter(|seg| seg.take_id != deleted)
        .collect();

    // The survivor that inherits the holes: what the engine's own fallback
    // would pick from the takes that remain.
    let mut survivors = group.clone();
    survivors.takes.retain(|t| t.id != deleted);
    let Some(filler) = crate::state::takes::fallback_take(&survivors) else {
        return kept;
    };

    let mut comp = Comp { segments: kept };
    for gap in gaps(&comp.segments, group.slot) {
        comp.promote(gap, filler);
    }
    comp.segments
}

/// The stretches of `slot` no segment covers, in order. `segments` is
/// assumed sorted and non-overlapping, which the comp helpers guarantee.
fn gaps(segments: &[CompSegment], slot: TimelineRange) -> Vec<TimelineRange> {
    let mut out = Vec::new();
    let mut cursor = slot.start;
    for seg in segments {
        if seg.range.start > cursor {
            out.push(TimelineRange::from_bounds(cursor, seg.range.start));
        }
        cursor = cursor.max(seg.range.end());
    }
    if cursor < slot.end() {
        out.push(TimelineRange::from_bounds(cursor, slot.end()));
    }
    out
}

// ---------------------------------------------------------------------------
// Audible extent
// ---------------------------------------------------------------------------

/// The stretch of timeline `take` can actually sound over, intersected
/// with its group's slot.
///
/// A take is **not** guaranteed to fill its slot. Cycle recording gives
/// pass 0 the punch-in point as its clip start, and any pass cut short at
/// stop ends before the loop end; a later trim shortens it again. The comp
/// renderer clamps a span to the clip and ramps at its real edge (todo
/// #409), so an over-long segment is silent rather than fatal — but it is
/// still a hole, which is why promotes are clamped to this.
///
/// **Known limitation.** `AudioEvent::TakeCaptured` carries no extent, and
/// a cycle-record pass reaches the app *only* through that event — no
/// `RecordingFinished` is emitted for it — so an audio take's clip is
/// normally absent from `Resonance::clips` and the true extent is unknown
/// here. In that case the slot is returned, which is exactly the region
/// the engine's own default cover uses, so the clamp never *removes*
/// coverage the engine would have played. It bites whenever the app does
/// know the clip (a bounced or reloaded take, and every case once the
/// engine reports the extent). Reported to the architect as a follow-up.
fn audible_extent(
    r: &Resonance,
    group: &TakeGroup,
    take: &resonance_common::Take,
) -> TimelineRange {
    let TakeContent::Audio { clip_ref } = take.content else {
        // A MIDI take carries its notes inline and contributes nothing to
        // the audio comp at all; the slot is its full extent.
        return group.slot;
    };
    match r.clips.iter().find(|c| c.id == clip_ref) {
        Some(clip) => intersect(
            TimelineRange::from_bounds(
                clip.start_sample,
                clip.start_sample + clip.duration_samples,
            ),
            group.slot,
        ),
        None => group.slot,
    }
}

/// The overlap of two ranges, empty when they do not meet.
fn intersect(a: TimelineRange, b: TimelineRange) -> TimelineRange {
    TimelineRange::from_bounds(a.start.max(b.start), a.end().min(b.end()))
}
