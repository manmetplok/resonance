//! Which in-flight load of a clip id is still wanted (code review
//! FU-A13e).
//!
//! `LoadClipFromWav` / `LoadTakeClipFromWav` publish from a worker some
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
//! withdraws the id's ticket; the worker publishes only while its ticket
//! is still the id's current one. So a delete cancels every load of the
//! id submitted before it, whatever order the workers finish in, and a
//! load submitted after it is unaffected.
//!
//! # The locking contract
//!
//! The same as the take-clip park's (`take_park.rs`), for the same
//! reason: [`ClipLoadTickets::redeem`] and [`ClipLoadTickets::withdraw`]
//! are called **while holding `ctx.clips.write()`**, so "the load is
//! cancelled" and "the clip is not in the list" are one atomic step for
//! the delete, and "the ticket is live" and "the clip is pushed" one for
//! the worker. Nothing acquires `ctx.clips` while holding this lock: the
//! one order is clips → tickets (→ park).

use std::collections::HashMap;

use parking_lot::Mutex;

use crate::types::ClipId;

#[derive(Default)]
struct Inner {
    /// Never reused within a session, so a withdrawn id's re-issue can
    /// never match a stale worker's ticket.
    next: u64,
    /// The one load of each id that may still publish.
    current: HashMap<ClipId, u64>,
}

/// The live load ticket of every clip id with a load in flight. Shared
/// between the engine thread and the clip-load workers.
#[derive(Default)]
pub(crate) struct ClipLoadTickets {
    inner: Mutex<Inner>,
}

impl ClipLoadTickets {
    /// A load of `clip_id` is being submitted: hand it the id's new
    /// ticket. An earlier load of the id still in flight is superseded —
    /// it will not publish.
    ///
    /// Engine thread, at submit.
    pub(crate) fn issue(&self, clip_id: ClipId) -> u64 {
        let mut inner = self.inner.lock();
        inner.next += 1;
        let ticket = inner.next;
        inner.current.insert(clip_id, ticket);
        ticket
    }

    /// A load finished (or failed): `true` when it is still the id's live
    /// load, i.e. it may publish. Either way it is no longer in flight, so
    /// a live ticket is retired here; a superseded one leaves the newer
    /// load's ticket alone.
    ///
    /// Worker; on the publish path, call while holding `ctx.clips.write()`.
    pub(crate) fn redeem(&self, clip_id: ClipId, ticket: u64) -> bool {
        let mut inner = self.inner.lock();
        if inner.current.get(&clip_id) == Some(&ticket) {
            inner.current.remove(&clip_id);
            true
        } else {
            false
        }
    }

    /// Cancel every load of `clip_id` in flight — `DeleteClip`.
    ///
    /// Call while holding `ctx.clips.write()`.
    pub(crate) fn withdraw(&self, clip_id: ClipId) {
        self.inner.lock().current.remove(&clip_id);
    }
}
