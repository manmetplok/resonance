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
//!    sends starts from
//!    [`TakeGroup::effective_comp`](resonance_common::TakeGroup::effective_comp)
//!    — the shared cover definition written out as segments — and is
//!    edited through the `resonance_common` comp helpers, which keep it
//!    sorted, non-overlapping and inside the slot.
//! 2. **A promote never offers a region its take cannot fill.** A take's
//!    audible extent is what it recorded, not the group's slot: a pass
//!    that punched in late, or was cut short at stop, covers less. Since
//!    todo #409 the renderer clamps such a span to the clip and ramps at
//!    its real edge rather than panicking — but the uncovered part of the
//!    segment is still a silent hole in the composite, so
//!    [`Take::audible_extent`](resonance_common::Take::audible_extent)
//!    clamps it away up front.
//!
//!    That clamp reads the extent the engine reported on the take itself
//!    (todo #1396), never a clip lookup: a cycle-record pass reaches the
//!    app only through `AudioEvent::TakeCaptured` and a project load
//!    restores take groups without their clips, so a take clip is *never*
//!    in `Resonance::clips` and a lookup could only ever fall back to the
//!    slot — which is exactly the blindness this rule exists to avoid.
//! 3. **A rejected selection is never mirrored.** `SetActiveTake` with a
//!    take the group does not hold is dropped by the engine with no echo,
//!    so the app validates before sending instead of assuming success.
//!
//! # A deletion is a command, not a comp edit (todo #1401)
//!
//! Deleting a take is the one message here that is **not** a `SetTakeComp`.
//! A take's recording is an ordinary `AudioClip` in the engine's shared
//! clip list, inaudible only because the published comp table marks it
//! *governed*; a comp that stops naming the take stops governing its clip,
//! and the "deleted" pass comes back **louder**, raw on the ordinary clip
//! path on top of the comp that replaced it (ba doc #292, reproduced at
//! peak 1.25 against 1.0). Only the engine can park the clip out of the
//! render's input, so the app sends
//! [`AudioCommand::RemoveTake`] / [`AudioCommand::RemoveTakeGroup`]
//! (todo #1397) and lets one command carry the take, the re-covered comp
//! and a cleared solo together.
//!
//! Two consequences:
//!
//! * **A group's last take is no longer refused** — it removes the *lane*.
//!   An empty group would keep its slot forever (todo #1392), draw as
//!   chrome with nothing to comp, and an empty comp is precisely the state
//!   in which the cover falls back to the most recent pass — the one just
//!   deleted.
//! * **The mirror re-covers through the same shared helper the engine
//!   uses**, [`TakeGroup::remove_take`], so the lane drawn this frame and
//!   the comp the engine publishes cannot disagree. The app carries no
//!   second definition of a removal.
//!
//! A restore (undo/redo) is no longer a limit: `RestoreTakeGroups` (todo
//! #1394) replaces the engine's whole store from the mirror, so it can
//! create a group the engine never captured and remove one it holds —
//! which is why `undo::snapshot::resync_take_comps` is gone (todo #1399).
//! Since todo #1397 that command reconciles the parked recordings in both
//! directions as well, so it is also the whole undo *and* redo path for a
//! deletion: an undo un-parks the take's clip and it is audible again, a
//! redo re-parks it. Nothing else has to be re-asserted after a removal.

use iced::Task;
use resonance_audio::types::AudioCommand;
use resonance_common::{CompSegment, SlotCover, TakeGroup, TakeGroupId, TakeId, TimelineRange};

use crate::message::Message;
use crate::Resonance;

/// Comping edits to a cycle-record take lane (design doc #165, epic #15,
/// todo #411).
///
/// Every variant names the [`TakeGroup`](resonance_common::TakeGroup) it
/// edits by the engine's `group_id` and resolves to at most one
/// `SetTakeComp` plus one `SetActiveTake`, applied optimistically to
/// [`TakeGroupState`](crate::state::TakeGroupState) and confirmed by the
/// engine's `TakeCompChanged` / `ActiveTakeChanged` echoes. Each is one
/// atomic undo entry.
///
/// A message that would change nothing — an unknown group or take, a
/// selection that is already current, a promote that lands outside the
/// chosen take's recorded audio — is refused **before** dispatch by
/// `Resonance::take_edit_is_refused`, so it spends no undo entry and bumps
/// no revision (the same rule ba todo #1261 established for a refused
/// chain reorder).
///
/// Expanding / collapsing a lane is deliberately *not* here: it is
/// transient view state carried by `UiMessage::ToggleTakeLane` (todo
/// #413), never an undo entry and never persisted.
#[derive(Debug, Clone)]
pub enum TakeMessage {
    /// Solo one take of `group_id` across the whole slot, overriding the
    /// comp — or clear the override with `None` so the comp plays again.
    ///
    /// Refused when the group does not hold `take_id`: the engine drops
    /// such a command silently and emits **no** echo (ba doc #292), so a
    /// rejected selection would otherwise leave the mirror asserting a
    /// solo that never happened.
    SetActiveTake {
        group_id: TakeGroupId,
        take_id: Option<TakeId>,
    },
    /// Cut the comp in two at the transport playhead, creating a boundary
    /// to promote against. Refused unless the playhead lies strictly
    /// inside the group's slot and inside a segment (a cut on an existing
    /// boundary changes nothing).
    SplitCompAtPlayhead { group_id: TakeGroupId },
    /// Promote `take_id` across `range`, replacing whatever covered it.
    ///
    /// `range` is a *request*: it is clamped to the group's slot and to
    /// the region the take's own recording actually spans before anything
    /// is sent, and the message is refused if nothing survives. The engine
    /// does not sanitise segment ranges, and a segment over a region its
    /// take cannot fill renders as silence inside the composite.
    PromoteTakeSegment {
        group_id: TakeGroupId,
        take_id: TakeId,
        range: TimelineRange,
    },
    /// Remove `take_id` from the lane and re-cover the slot from the takes
    /// that remain.
    ///
    /// **A group's last take removes the lane** (ba todo #1401): an empty
    /// group keeps its slot forever, has nothing to comp or draw, and an
    /// empty comp is exactly the state in which the cover falls back to
    /// the most recent pass — the take just deleted. Refused only when the
    /// group or the take is not there.
    ///
    /// Sent to the engine as `RemoveTake` / `RemoveTakeGroup` rather than
    /// as a comp edit, because only the engine can park the take's
    /// recording out of the render — an un-parked one plays raw, at full
    /// gain, on the ordinary clip path.
    DeleteTake {
        group_id: TakeGroupId,
        take_id: TakeId,
    },
}

impl TakeMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // Take-lane comping (doc #165, todo #411). Every variant is one
            // discrete, atomic edit — there is no gesture here; the canvas drag
            // that *chooses* a promote range is todo #414's and commits by
            // emitting a single `PromoteTakeSegment`. Take groups ride the
            // `ProjectFile` snapshot (todo #412), so the generic Record path
            // reverses a comp edit with no per-message capture; the engine is
            // driven back by `replay_take_groups`'s `RestoreTakeGroups` on both
            // restore paths (todo #1394).
            //
            // An edit that would change nothing never reaches here: it is
            // dropped by `take_edit_is_refused` before `record_undo` runs, so no
            // vacuous entry is recorded.
            Self::SetActiveTake { .. }
            | Self::SplitCompAtPlayhead { .. }
            | Self::PromoteTakeSegment { .. }
            | Self::DeleteTake { .. } => UndoAction::Record,
        }
    }
}

/// The mutation a [`TakeMessage`] performs, resolved against current
/// state.
///
/// A comp edit and a removal are different *kinds* of edit rather than
/// different fields of one, because they reach the engine by different
/// routes: a comp edit is a value the app computes and pushes, a removal
/// is a command the engine executes (parking the recording, which the app
/// cannot do) and echoes back.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TakeEdit {
    /// Adopt and push a comp cover and/or an active take on a group that
    /// survives the edit. `None` means "leave alone / send nothing";
    /// `active_take: Some(None)` clears the solo.
    Comp {
        group_id: TakeGroupId,
        comp: Option<Vec<CompSegment>>,
        active_take: Option<Option<TakeId>>,
    },
    /// Remove one take from a group that keeps at least one other.
    RemoveTake {
        group_id: TakeGroupId,
        take_id: TakeId,
    },
    /// Remove the whole lane, because the take asked for was its last.
    RemoveGroup { group_id: TakeGroupId },
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
    match edit {
        TakeEdit::Comp {
            group_id,
            comp,
            active_take,
        } => {
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
        // One command, not three. `RemoveTake` drops the take, re-covers
        // the slot from the survivors and clears a solo that named it —
        // and, decisively, parks the take's recording out of the render's
        // input, which no comp the app could push does. The mirror runs
        // the *same* `TakeGroup::remove_take` the handler runs, so the
        // `TakeRemoved` / `TakeCompChanged` / `ActiveTakeChanged` echoes
        // behind the command re-apply values the lane already shows.
        TakeEdit::RemoveTake { group_id, take_id } => {
            r.take_groups.remove_take(group_id, take_id);
            let _ = r
                .engine
                .send(AudioCommand::RemoveTake { group_id, take_id });
        }
        TakeEdit::RemoveGroup { group_id } => {
            r.take_groups.remove_group(group_id);
            let _ = r.engine.send(AudioCommand::RemoveTakeGroup { group_id });
        }
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
    Some(TakeEdit::Comp {
        group_id,
        comp: None,
        active_take: Some(take_id),
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
    let mut comp = group.effective_comp();
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
    let target = intersect(range, take.audible_extent(group.slot));
    if target.is_empty() {
        return None;
    }
    let mut comp = group.effective_comp();
    comp.promote(target, take_id, SlotCover::of(group));
    finish_comp_edit(group, comp.segments)
}

/// A deletion is refused only when there is nothing to delete — an unknown
/// group, or a take the group does not hold.
///
/// # Take or lane
///
/// **The last take takes the lane with it** (todo #1397, ba doc #292). The
/// choice is `group.takes.len() == 1`, made here as well as in the engine's
/// `handle_remove_take` because each side owns a store of groups and each
/// has to drop the group from its own; the rule itself lives on
/// [`TakeGroup::remove_take`], which is why neither side re-derives what a
/// removal *does*.
///
/// Sending `RemoveTakeGroup` rather than leaning on the engine's identical
/// redirect keeps the command the app sends equal to the decision the app
/// already made when it dropped the lane from the mirror — the echo is
/// `TakeGroupRemoved` either way.
fn plan_delete(r: &Resonance, group_id: TakeGroupId, take_id: TakeId) -> Option<TakeEdit> {
    let group = r.take_groups.group(group_id)?;
    group.take(take_id)?;
    Some(if group.takes.len() == 1 {
        TakeEdit::RemoveGroup { group_id }
    } else {
        TakeEdit::RemoveTake { group_id, take_id }
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
///
/// That is only safe because the cover the caller edited was materialized
/// from `effective_comp`, which honours the **active take** as its first
/// tier (todo #1395). Before that, a split taken while take 0 was soloed
/// materialized the *fallback* take's cover and then dropped the solo: the
/// user was hearing take 0, hit a gesture that names no take at all, and
/// came out hearing the latest pass. Seeding from the active take is what
/// makes solo → split → promote the natural comping flow.
fn finish_comp_edit(group: &TakeGroup, segments: Vec<CompSegment>) -> Option<TakeEdit> {
    let comp_changed = segments != group.comp.segments;
    let clear_solo = group.active_take.is_some();
    if !comp_changed && !clear_solo {
        return None;
    }
    Some(TakeEdit::Comp {
        group_id: group.id,
        comp: comp_changed.then_some(segments),
        active_take: clear_solo.then_some(None),
    })
}

/// The overlap of two ranges, empty when they do not meet.
fn intersect(a: TimelineRange, b: TimelineRange) -> TimelineRange {
    TimelineRange::from_bounds(a.start.max(b.start), a.end().min(b.end()))
}
