//! Which in-flight load of a clip id is still wanted (code review
//! FU-A13e).
//!
//! `LoadClipFromWav` / `LoadTakeClipFromWav` decode on a worker some
//! milliseconds after the command (`clips::submit_clip_load`). An undo /
//! redo burst over a clip add sends `LoadClipFromWav(id)`,
//! `DeleteClip(id)`, `LoadClipFromWav(id)` inside that window, and before
//! this the delete had nothing to act on: it was parked behind the first
//! load (ba doc #276 BUG 1's deferral), and whichever load reached the
//! clip list first won the duplicate check. Depending on the order the
//! two workers and the engine loop's replay ran in, the engine ended with
//! no clip, or with the *first* load's clip (whose delete had been
//! applied to the second one's).
//!
//! A ticket per submitted load settles it. Submitting a load issues a
//! fresh ticket for the id, superseding any earlier one; `DeleteClip`
//! withdraws the id's ticket; a finished load is applied only while its
//! ticket is still the id's current one. So a delete cancels every load of
//! the id submitted before it, whatever order the workers finish in, and a
//! load submitted after it is unaffected.
//!
//! # Engine-thread only (code review ARCH-02 B-5)
//!
//! Every method runs on the engine control thread. The worker never
//! touches the tickets: it posts its result
//! ([`EngineInternal::ClipLoaded`](super::internal::EngineInternal) /
//! `ClipLoadFailed`, carrying the ticket it was issued), and the engine
//! thread redeems it in the same step that checks the clip list and
//! publishes the clip. Until B-5 the worker redeemed under
//! `ctx.clips.write()` and `DeleteClip` withdrew under the same lock, so
//! "the load is cancelled" and "the clip is not in the list" were one
//! atomic step; now both are engine-thread sequencing — a `DeleteClip` is
//! applied wholly before or wholly after a `ClipLoaded`, never between its
//! redeem and its publish — so the same two orderings are the only ones.

use std::collections::HashMap;

use crate::types::ClipId;

/// The live load ticket of every clip id with a load in flight. Owned by
/// the engine thread's `HandlerState`.
#[derive(Default)]
pub(crate) struct ClipLoadTickets {
    /// Never reused within a session, so a withdrawn id's re-issue can
    /// never match a stale worker's ticket.
    next: u64,
    /// The one load of each id that may still publish.
    current: HashMap<ClipId, u64>,
}

impl ClipLoadTickets {
    /// A load of `clip_id` is being submitted: hand it the id's new
    /// ticket. An earlier load of the id still in flight is superseded —
    /// it will not publish.
    pub(crate) fn issue(&mut self, clip_id: ClipId) -> u64 {
        self.next += 1;
        let ticket = self.next;
        self.current.insert(clip_id, ticket);
        ticket
    }

    /// A load finished (or failed): `true` when it is still the id's live
    /// load, i.e. it may publish. Either way it is no longer in flight, so
    /// a live ticket is retired here; a superseded one leaves the newer
    /// load's ticket alone.
    pub(crate) fn redeem(&mut self, clip_id: ClipId, ticket: u64) -> bool {
        if self.current.get(&clip_id) == Some(&ticket) {
            self.current.remove(&clip_id);
            true
        } else {
            false
        }
    }

    /// Cancel every load of `clip_id` in flight — `DeleteClip`.
    pub(crate) fn withdraw(&mut self, clip_id: ClipId) {
        self.current.remove(&clip_id);
    }
}
