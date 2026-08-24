//! Take groups on the engine control thread: the authoritative
//! `TakeGroup` store, the two comp-editing command handlers, and the
//! flatten-and-publish step that makes a comp audible (epic #15, doc #165,
//! todo #409).
//!
//! The engine owns the take groups because it is the only thing that can
//! see a loop pass finish: `transport::finalize_loop_record_pass` calls
//! [`store_take`] as each pass rolls, so the group exists before the app
//! has heard about it. The app then edits the comp through
//! `AudioCommand::SetTakeComp` / `SetActiveTake` and is told what stuck via
//! the `TakeCompChanged` / `ActiveTakeChanged` echoes — the one-way
//! command/event boundary of doc #105, with no read-getter on
//! `AudioEngine`.
//!
//! Everything here runs on the control thread and may allocate. The audio
//! thread only ever reads the flattened [`CompRenderTable`] that
//! [`publish_take_comp`] stores into `SharedState::take_comp`.

use std::sync::Arc;

use resonance_common::{
    CompSegment, Take, TakeContent, TakeGroup, TakeGroupId, TakeId, TimelineRange,
};

use crate::types::{AudioEvent, TrackId};

use super::thread::{HandlerCtx, HandlerState};

/// Record a freshly-captured take into the authoritative take-group store,
/// creating the group (bound to `slot`) on its first take. Returns the
/// engine-assigned [`TakeId`], which the app mirrors so later comp /
/// active-take commands line up with what the engine renders.
///
/// The id is allocated from the group itself — one past the highest it
/// already holds — rather than derived from `pass_index`. Deriving it would
/// be sound only if a `(group, pass)` pair could never emit twice, and it
/// can: `finalize_loop_record_pass` runs an audio loop and a MIDI loop that
/// both resolve to the *same* group for the same track, so a track present
/// in both would get two takes sharing one id (ba doc #292). Allocating per
/// group makes "unique within its group" true by construction, whatever the
/// capture path does.
pub(crate) fn store_take(
    state: &mut HandlerState,
    group_id: TakeGroupId,
    track_id: TrackId,
    slot: TimelineRange,
    pass_index: u32,
    content: &TakeContent,
) -> TakeId {
    let group = state
        .take_groups
        .entry(group_id)
        .or_insert_with(|| TakeGroup::new(group_id, track_id, slot));
    group.slot = slot;
    push_take(group, slot, pass_index, content)
}

/// Append one take to `group`, rebinding it to `slot`, and return the id
/// allocated for it: one past the highest id the group already holds, or
/// `0` when it is empty.
///
/// Allocating from the group rather than from a counter kept beside it
/// means the allocator cannot drift out of step with the takes actually
/// present — including for a group rehydrated from a saved project, where
/// a separate counter would have to be persisted too.
///
/// Split out from [`store_take`] as the pure half (no `HandlerState`), so
/// `tests/loop_record_takes.rs` can pin the uniqueness invariant against
/// the real code rather than a re-implementation of it.
pub fn push_take(
    group: &mut TakeGroup,
    slot: TimelineRange,
    pass_index: u32,
    content: &TakeContent,
) -> TakeId {
    group.slot = slot;
    let take_id = group
        .takes
        .iter()
        .map(|t| t.id)
        .max()
        .map_or(0, |highest| highest + 1);
    group.add_take(Take::new(take_id, pass_index, 0, content.clone()));
    take_id
}

/// Flatten the authoritative take groups into the audio-thread-visible
/// comp playback table and publish it wait-free. Called whenever a group is
/// captured, comped, or has its active take changed — the single point at
/// which an edit becomes audible, on both the live and the bounce path.
pub(crate) fn publish_take_comp(ctx: &HandlerCtx, state: &HandlerState) {
    let table = crate::mixer::build_comp_table(&state.take_groups);
    ctx.shared.take_comp.store(Arc::new(table));
}

/// Replace the comp of take group `group_id`, republish the playback table
/// so the new cover plays and bounces, and echo `TakeCompChanged`. An
/// unknown group is ignored (the handlers' standing missing-lookup
/// convention).
pub(crate) fn handle_set_take_comp(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    group_id: TakeGroupId,
    segments: Vec<CompSegment>,
) {
    let Some(group) = state.take_groups.get_mut(&group_id) else {
        return;
    };
    group.comp.segments = segments;
    let echo = group.comp.segments.clone();
    publish_take_comp(ctx, state);
    let _ = ctx.event_tx.send(AudioEvent::TakeCompChanged {
        group_id,
        segments: echo,
    });
}

/// Set (or clear) the active take of group `group_id`, republish the
/// playback table, and echo `ActiveTakeChanged`. An unknown group — or a
/// `Some(take_id)` naming a take the group does not hold — is ignored, so a
/// stale selection can never silence a group that would otherwise play its
/// comp.
pub(crate) fn handle_set_active_take(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    group_id: TakeGroupId,
    take_id: Option<TakeId>,
) {
    let Some(group) = state.take_groups.get_mut(&group_id) else {
        return;
    };
    if let Some(tid) = take_id {
        if group.take(tid).is_none() {
            return;
        }
    }
    group.active_take = take_id;
    publish_take_comp(ctx, state);
    let _ = ctx
        .event_tx
        .send(AudioEvent::ActiveTakeChanged { group_id, take_id });
}
