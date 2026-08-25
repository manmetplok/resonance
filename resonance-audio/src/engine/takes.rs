//! Take groups on the engine control thread: the authoritative
//! `TakeGroup` store, the two comp-editing command handlers, and the
//! flatten-and-publish step that makes a comp audible (epic #15, doc #165,
//! todo #409).
//!
//! The engine owns the take groups because it is the only thing that can
//! see a loop pass finish: `transport::finalize_loop_record_pass` calls
//! [`capture_take_event`] as each pass rolls, so the group exists before
//! the app has heard about it. The app then edits the comp through
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

/// The authoritative take-group store: every [`TakeGroup`] the engine
/// holds, keyed by id. Named so the pure halves of the handlers below can
/// be driven from `tests/` without a `HandlerState`.
pub type TakeGroupStore = std::collections::HashMap<TakeGroupId, TakeGroup>;

/// The existing take group on `track_id` that a cycle-record run over
/// `slot` belongs to, or `None` when the run opens a genuinely new lane.
///
/// # The ruling: what "the same slot" means (todo #1392)
///
/// **One lane per slot.** Takes accumulate into a single [`TakeGroup`] for
/// a given track + loop region however many times record is pressed —
/// Logic / Pro Tools / Reaper take-folder behaviour. Three passes, stop,
/// two more over the same region is *one* lane of five takes. That makes
/// "which group?" a lookup against the store rather than against the
/// in-flight `LoopRecordSession`,
/// which is why it lives here: `state.take_groups` outlives a record run
/// and holds groups rehydrated from a saved project (todo #1394), so a run
/// over a slot that came back from disk joins *that* lane too.
///
/// The judgement call is what counts as "the same" region, and there are
/// two halves to it. They are not independent: because a lane's region
/// never moves (below), the matching rule decides how much freshly
/// recorded audio a joining run can leave outside its own lane — so it has
/// to be **tight**, not merely sound.
///
/// ## Matching: near-exact, with one crossfade of slack
///
/// Exact [`TimelineRange`] equality very nearly works. The loop points a
/// run records over come from `state.rec.loop_in/loop_out`, so pressing
/// record twice without touching the loop gives bit-identical ranges. What
/// equality cannot absorb is *jitter*: a bar-snapped region re-derived a
/// frame out (a tempo-map rounding, a different path to the same bar)
/// would silently fork a second lane over what the user sees as one
/// region.
///
/// So the rule is: the two regions are the same slot when **both endpoints
/// agree to within [`SAME_SLOT_TOLERANCE_FRAMES`]** — one comp crossfade
/// window. Anything further apart is a different lane.
///
/// The tolerance is deliberately far tighter than "the user probably meant
/// the same section". A looser rule (a majority overlap, or an
/// intersection-over-union threshold) accepts materially different regions
/// — 4 bars against 7 — and, since the lane keeps the *first* region, the
/// three-bar tail of the new take is then covered by nothing:
/// `build_comp_table` resolves spans against `group.slot`, and the take's
/// clip is in `governed_clips`, so it cannot play on the ordinary clip path
/// either. That is a take silently lost, which doc #165 forbids outright.
/// Bounding the disagreement in *absolute frames* rather than as a
/// proportion is what makes the guarantee hold for a 32-bar loop as well as
/// a 1-bar one.
///
/// Two things make 256 frames a safe size for that bound — and note that
/// "downstream can't tell the difference" is **not** one of them. The
/// render path resolves single frames; a seam crossfade is
/// `COMP_XFADE_FRAMES / 2` = 128 per side and
/// [`CLIP_DECLICK_FRAMES`](crate::mixer::CLIP_DECLICK_FRAMES) is 96, both
/// *smaller* than the tolerance, and the declick is precisely the ramp
/// that fires at the edge in question. What actually holds is:
///
/// 1. **Both within-tolerance outcomes are milder instances of a state
///    that is already handled and already disclosed.** A joining run
///    ending a little short leaves its clip short of the lane's slot;
///    `mix_track_comp` clamps the span to the clip's real extent and
///    declicks, and the lane draws the flat "no audio here" line over the
///    rest. That happens by far more than 256 frames on every punch-in
///    pass and every pass truncated by stop. Since todo #1396 the take
///    even carries the measurement: it records its own `extent`, and
///    [`Take::audible_extent`](resonance_common::Take::audible_extent)
///    intersects that with the lane's slot, so the surplus is clamped
///    rather than mis-placed. The residual *undisclosed* cost is 5.3 ms
///    at an edge, bounded absolutely — around 1/500th of what the
///    majority-overlap rule could lose, and
///    `a_within_tolerance_join_strands_at_most_the_tolerance` measures it
///    rather than taking the bound on trust.
/// 2. **Every UI path that sets the loop region snaps to the tempo grid.**
///    `UpdateLoopDrag` routes through `snap_sample_to_grid_tempo`, whose
///    finest step is one beat: 24 000 frames at 120 bpm, and still ~2 900
///    at an absurd 999 bpm. The smallest *deliberate* change a user can
///    make to a loop is one to two orders of magnitude above this
///    tolerance, so the band between "jitter" and "meant it" is wide and
///    empty. That is the headroom argument, and it is the one to re-check
///    before anyone widens this constant.
///
/// Cases:
///
/// - identical, or nudged by a frame → same lane;
/// - loop lengthened from 4 bars to 7 → **new lane**, and it plays in full;
/// - loop moved by a 16th note → new lane;
/// - a short loop inside a long one, or two abutting regions → new lane.
///
/// Ties are broken deterministically (least endpoint drift, then lowest id)
/// so a `HashMap`'s iteration order cannot change the answer. Two lanes
/// within a tolerance of each other cannot arise from recording — the
/// second run would have joined the first — but a saved project can hold
/// any pair of slots, so the tie-break is load-bearing rather than
/// theoretical.
///
/// ## Rebinding: a lane's region never moves
///
/// [`store_take_in`] used to do `group.slot = slot` on every take. Reusing a
/// group would then let a nudged loop region silently *move* the lane —
/// and every [`CompSegment`] the user has already drawn is expressed in
/// absolute timeline positions against the old one, so segments would fall
/// outside their own slot, `is_full_cover` would start lying, and
/// `effective_cover`'s seeding (todo #1395) would be seeded off a region
/// its own comp no longer describes.
///
/// That is the whole of the argument, and it is enough. It is *not* the
/// case that a grown slot would make a short take claim material it never
/// recorded: `mix_track_comp` intersects every span with the clip's real
/// post-trim extent and skips it when empty, and the lane draws each take's
/// `audible_extent`. Both were true before this todo and neither is relied
/// on here — but an over-stated reason is how a future change talks itself
/// into rebinding, so it is worth being exact about which one holds.
///
/// Rebinding is therefore **not** safe, and this code does not do it. The
/// run that creates a lane fixes its region; later runs join it and leave
/// it alone.
pub fn take_group_for_slot(
    take_groups: &TakeGroupStore,
    track_id: TrackId,
    slot: TimelineRange,
) -> Option<TakeGroupId> {
    take_groups
        .values()
        .filter(|g| g.track_id == track_id && slots_match(g.slot, slot))
        // Deterministic under `HashMap` iteration order: the closest lane
        // wins, lowest id breaks a tie. The key is unique per group, so
        // iteration order cannot reach the answer.
        .min_by_key(|g| (endpoint_drift(g.slot, slot), g.id))
        .map(|g| g.id)
}

/// The take group a cycle-record pass over `slot` on `track_id` belongs
/// to, together with **that group's own slot** — which is what every take
/// and every `TakeCaptured` event must then carry, so the engine's store
/// and the app's mirror stay identical.
///
/// One lane per slot (todo #1392): an existing lane is reused via
/// [`take_group_for_slot`], and a fresh [`TakeGroupId`] comes off
/// `next_group_id` only when the run opens a genuinely new one. Because
/// the lookup is against the store rather than the in-flight record
/// session, this covers all three cases uniformly — a later pass of the
/// same run, a later run in the same session, and a run over a lane
/// rehydrated from a saved project (todo #1394).
///
/// Within a run no per-session cache is needed: the first pass's
/// [`store_take_in`] inserts the group, so every later pass — and the MIDI
/// half of the *same* pass — finds it here by lookup.
pub fn resolve_take_group(
    take_groups: &TakeGroupStore,
    next_group_id: &mut TakeGroupId,
    track_id: TrackId,
    slot: TimelineRange,
) -> (TakeGroupId, TimelineRange) {
    if let Some(id) = take_group_for_slot(take_groups, track_id, slot) {
        return (id, take_groups[&id].slot);
    }
    let id = *next_group_id;
    *next_group_id += 1;
    (id, slot)
}

/// How far apart two loop regions may sit and still count as the same
/// slot: one comp crossfade window at each endpoint (~5.3 ms at 48 kHz).
///
/// Expressed as [`COMP_XFADE_FRAMES`](crate::mixer::COMP_XFADE_FRAMES) to
/// borrow a number of the right order rather than invent one; the comp
/// machinery does **not** make a difference this small unobservable (the
/// declick ramp is shorter still, at 96 frames). What justifies the size
/// is the grid the loop region is set on — the finest snap is one beat,
/// two orders of magnitude wider — and the fact that what it can strand
/// is bounded in *absolute* frames rather than as a share of the loop.
/// [`take_group_for_slot`] carries the argument in full; read it before
/// changing this.
pub const SAME_SLOT_TOLERANCE_FRAMES: u64 = crate::mixer::COMP_XFADE_FRAMES;

/// Total disagreement between two regions: how far the starts are apart
/// plus how far the ends are. Used both as the match test and, because it
/// is unique per candidate lane, as the tie-break key.
fn endpoint_drift(a: TimelineRange, b: TimelineRange) -> u64 {
    a.start.abs_diff(b.start) + a.end().abs_diff(b.end())
}

/// Whether two loop regions are "the same slot" — both endpoints agree to
/// within [`SAME_SLOT_TOLERANCE_FRAMES`]. See [`take_group_for_slot`] for
/// the reasoning; this is the predicate that ruling reduces to.
///
/// Empty ranges match only their exact selves. A cycle-record slot is
/// never empty (`begin_recording_stream` requires `loop_out > loop_in`),
/// but a restored group's could be anything, and letting a zero-length
/// region absorb takes recorded near it would be a silent loss of the
/// entire take rather than of an edge.
pub fn slots_match(a: TimelineRange, b: TimelineRange) -> bool {
    if a.is_empty() || b.is_empty() {
        return a == b;
    }
    a.start.abs_diff(b.start) <= SAME_SLOT_TOLERANCE_FRAMES
        && a.end().abs_diff(b.end()) <= SAME_SLOT_TOLERANCE_FRAMES
}

/// File one freshly-captured cycle-record take and return the
/// `TakeCaptured` event announcing it — resolving the lane it belongs to,
/// storing it, and building the event, in one place.
///
/// This is the *whole* of the engine-side capture glue.
/// `finalize_loop_record_pass` calls it once per rolled audio take and once
/// per captured MIDI take and does nothing with the result but send it,
/// which is deliberate: the three values that must come from the **group**
/// rather than from the run — the group id, the take id and the slot —
/// were previously copied into the event by hand at each call site, where
/// nothing could tell if one of them drifted back to the run's value. With
/// one site there is nothing left to drift, and `tests/loop_record_takes.rs`
/// asserts on the event this returns rather than on a re-implementation of
/// how it is assembled.
///
/// `run_slot` is the region the *run* is cycling over. The event carries
/// the lane's own slot, which differs when the run joined an existing lane
/// whose region is a few frames off — see [`take_group_for_slot`].
///
/// `extent` is passed through untouched: it is what this pass really
/// recorded (ba todo #1396), a property of the run and not of the lane, so
/// a joining take reports its own and `Take::audible_extent` reconciles
/// the two.
pub fn capture_take_event(
    take_groups: &mut TakeGroupStore,
    next_group_id: &mut TakeGroupId,
    track_id: TrackId,
    run_slot: TimelineRange,
    extent: TimelineRange,
    content: TakeContent,
) -> AudioEvent {
    let (group_id, slot) = resolve_take_group(take_groups, next_group_id, track_id, run_slot);
    let (take_id, pass_index) =
        store_take_in(take_groups, group_id, track_id, slot, extent, &content);
    AudioEvent::TakeCaptured {
        group_id,
        take_id,
        track_id,
        slot,
        pass_index,
        extent,
        content,
    }
}

/// Record a freshly-captured take into the authoritative take-group store,
/// creating the group (bound to `slot`) on its first take. Returns the
/// engine-assigned [`TakeId`] and the take's ordinal within its group,
/// both of which the app mirrors — the id so later comp / active-take
/// commands line up with what the engine renders, the ordinal so the lane
/// stacks and labels its takes in capture order.
///
/// Both are allocated from the group itself rather than from the record
/// run. For the id, deriving it from the pass would be sound only if a
/// `(group, pass)` pair could never emit twice, and it can:
/// `finalize_loop_record_pass` runs an audio loop and a MIDI loop that both
/// resolve to the *same* group for the same track, so a track present in
/// both would get two takes sharing one id (ba doc #292).
///
/// `slot` binds a *newly created* group; it never rebinds an existing one.
/// See [`take_group_for_slot`] for why a lane's region is immutable once
/// its first take has landed — this is the writer that used to break it,
/// which is why `tests/loop_record_takes.rs` pins it here as well as on
/// [`resolve_take_group`]'s return.
pub fn store_take_in(
    take_groups: &mut TakeGroupStore,
    group_id: TakeGroupId,
    track_id: TrackId,
    slot: TimelineRange,
    extent: TimelineRange,
    content: &TakeContent,
) -> (TakeId, u32) {
    let group = take_groups
        .entry(group_id)
        .or_insert_with(|| TakeGroup::new(group_id, track_id, slot));
    push_take(group, extent, content)
}

/// Append one take to `group` and return the id allocated for it and its
/// ordinal within the group — each one past the highest the group already
/// holds, or `0` when it is empty.
///
/// **The ordinal is not the loop pass.** It is stored (and mirrored) as
/// `Take::pass_index`, and while a group was one record run the two were
/// the same thing; since todo #1392 a group spans runs, and a run's
/// `pass_index` restarts at 0 at every record press. The app sorts a lane's
/// takes by `(pass_index, id)` and labels them `T{pass_index + 1}`, so
/// emitting the run's counter made a five-take lane stack `0,3,1,4,2` and
/// read `T1, T1, T2, T2, T3`. Deriving it here — from the group, like the
/// id — makes both correct by construction. The run's own counter stays on
/// [`LoopRecordSession`](super::thread::LoopRecordSession), where it is
/// still exactly right for what it is used for: whether this is the
/// punch-in pass.
///
/// `extent` is what the pass really recorded over —
/// [`RolledAudioTake::extent`](crate::recording::RolledAudioTake::extent)
/// for an audio take, the run's slot for a MIDI one. It is stored on the
/// take (and so persisted, and so echoed to the app on both the capture
/// and the restore path) because the app never holds a take's clip and
/// could not otherwise tell a punched-in pass from one that filled its
/// slot; see [`resonance_common::Take::audible_extent`] (ba todo #1396).
///
/// It is the run's own measurement, not the lane's: a take that joined a
/// lane whose region sits a few frames off records what *it* captured, and
/// `audible_extent` intersects that with the slot. That is what makes the
/// tolerance in [`take_group_for_slot`] safe to have at all.
///
/// Both ids allocate from `max + 1` rather than `takes.len()` so that
/// removing a take (todo #1397) cannot make the next one collide with a
/// survivor.
///
/// **A knock-on, and it is intended.** The audio roll and the MIDI capture
/// of *one* pass on a dual-armed track now get **different** ordinals (0
/// and 1) where they used to share the run's. That is the wanted answer:
/// the ordinal is a lane *row* now, the two takes occupy two rows, and two
/// rows must not both be labelled `T1` — a shared label is unresolvable in
/// the comp ribbon, the SOLO tag and the track-header list, which all read
/// it. `resonance-app/tests/timeline/take_group_mirror.rs` keeps feeding
/// the mirror a hand-made collision anyway, because the mirror must key on
/// the take id whatever policy runs here.
///
/// Does **not** touch `group.slot`: a lane's region is fixed by the run
/// that created it (todo #1392 — see [`take_group_for_slot`]).
///
/// Split out from [`store_take_in`] so
/// `tests/loop_record_takes.rs` can pin the uniqueness invariants against
/// the real code rather than a re-implementation of them.
pub fn push_take(
    group: &mut TakeGroup,
    extent: TimelineRange,
    content: &TakeContent,
) -> (TakeId, u32) {
    let next_from = |highest: Option<u64>| highest.map_or(0, |h| h + 1);
    let take_id = next_from(group.takes.iter().map(|t| t.id).max());
    let pass_index = next_from(group.takes.iter().map(|t| u64::from(t.pass_index)).max()) as u32;
    group.add_take(Take::new(take_id, pass_index, 0, extent, content.clone()));
    (take_id, pass_index)
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
///   and hands the take straight to [`store_take_in`]). So a project whose
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
    take_groups: &mut TakeGroupStore,
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
