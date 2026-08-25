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

use crate::types::{AudioEvent, ClipId, TrackId};

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
    extent: TimelineRange,
    content: &TakeContent,
) -> TakeId {
    let group = state
        .take_groups
        .entry(group_id)
        .or_insert_with(|| TakeGroup::new(group_id, track_id, slot));
    group.slot = slot;
    push_take(group, slot, pass_index, extent, content)
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
/// `extent` is what the pass really recorded over —
/// [`RolledAudioTake::extent`](crate::recording::RolledAudioTake::extent)
/// for an audio take, the slot for a MIDI one. It is stored on the take
/// (and so persisted, and so echoed to the app on both the capture and the
/// restore path) because the app never holds a take's clip and could not
/// otherwise tell a punched-in pass from one that filled its slot; see
/// [`resonance_common::Take::audible_extent`] (ba todo #1396).
///
/// Split out from [`store_take`] as the pure half (no `HandlerState`), so
/// `tests/loop_record_takes.rs` can pin the uniqueness invariant against
/// the real code rather than a re-implementation of it.
pub fn push_take(
    group: &mut TakeGroup,
    slot: TimelineRange,
    pass_index: u32,
    extent: TimelineRange,
    content: &TakeContent,
) -> TakeId {
    group.slot = slot;
    let take_id = group
        .takes
        .iter()
        .map(|t| t.id)
        .max()
        .map_or(0, |highest| highest + 1);
    group.add_take(Take::new(take_id, pass_index, 0, extent, content.clone()));
    take_id
}

/// Replace `take_groups` wholesale with `groups`, raising `next_group_id`
/// above every restored group id **and** `next_clip_id` above every audio
/// take's `clip_ref`. The pure half of [`handle_restore_take_groups`] — no
/// `HandlerState`, no publish — so `tests/loop_record_takes.rs` can pin the
/// store/allocator contract against the real code.
///
/// Three invariants live here:
///
/// - **Replace, don't merge.** Both senders rebuild the app-side mirror
///   from scratch before sending, so replacing keeps the two identical;
///   merging would resurrect takes an undo had just deleted.
/// - **Both allocators only ever rise.** Taking `max` rather than assigning
///   means a restore can never hand a *later* cycle-record run an id that
///   an earlier one in the same session already used — which a plain
///   `= highest + 1` would do when a project holding one group is loaded
///   on top of a session that had already recorded several.
/// - **An audio take's `clip_ref` is a reservation, not just a reference**
///   (ba todo #1393). `ClearAll` resets `next_clip_id` to 1 on every
///   project load, and the only paths that push it back past a restored id
///   are `LoadClipFromWav` and its MIDI twin — which *timeline* clips take
///   and take clips never do (`roll_audio_pass` writes `audio/clip_N.wav`
///   and hands the take straight to [`store_take`]). So a project whose
///   takes hold `clip_ref` 100..102 while its timeline clips stop at 5
///   reopened with `next_clip_id = 6`, and the next recording or import
///   **overwrote `audio/clip_100.wav`** — the take then silently played the
///   new material. Same shape as the media pool's `ReserveAssetIds` (ba doc
///   #276 BUG 2), reserved here rather than through a second command
///   because this is the one path that already carries every restored
///   `clip_ref`, so it cannot go out of step with what was restored.
///
/// MIDI takes need nothing analogous: `TakeContent::Midi` carries its notes
/// inline and names no clip and no file, so it consumes no id from either
/// allocator (`finalize_loop_record_pass`'s MIDI half never touches
/// `next_clip_id`).
///
/// Take ids need no reservation either: [`push_take`] derives them from the
/// group's own takes, so a restored group allocates correctly the moment
/// it is present.
pub fn restore_take_groups_in_place(
    take_groups: &mut std::collections::HashMap<TakeGroupId, TakeGroup>,
    next_group_id: &mut TakeGroupId,
    next_clip_id: &mut ClipId,
    groups: Vec<TakeGroup>,
) {
    take_groups.clear();
    for group in groups {
        *next_group_id = (*next_group_id).max(group.id + 1);
        for take in &group.takes {
            if let TakeContent::Audio { clip_ref } = take.content {
                *next_clip_id = (*next_clip_id).max(clip_ref + 1);
            }
        }
        take_groups.insert(group.id, group);
    }
}

/// Rehydrate the take-group store from a loaded project (or an undo
/// snapshot), reserve the ids the restored takes hold, and republish the
/// comp table, so a restored comp plays and bounces without waiting for a
/// transport change (ba todo #1394, #1393).
///
/// Silent by design: the app is the sender *and* the mirror here, so there
/// is nothing to tell it that it did not just say. That is also why the
/// clip-id reservation rides along on this command rather than echoing
/// anything back — a freshly loaded project must not come up dirty.
pub(crate) fn handle_restore_take_groups(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    groups: Vec<TakeGroup>,
) {
    restore_take_groups_in_place(
        &mut state.take_groups,
        &mut state.next_take_group_id,
        &mut state.next_clip_id,
        groups,
    );
    publish_take_comp(ctx, state);
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
