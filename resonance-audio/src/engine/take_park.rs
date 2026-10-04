//! Where a removed take's recording goes — and how a removal wins the race
//! against a take-clip load that has not landed yet (ba todo #1397, #1403).
//!
//! # How a removal and an in-flight load agree
//!
//! A take's recording is an ordinary [`AudioClip`] in the engine's clip
//! list, and it is inaudible only because the published comp table
//! marks it *governed*. Removing the take stops it being governed, so
//! `takes::park_take_clip` lifts the clip **out of** the list: taking it
//! out of the render's input cannot be forgotten out of a skip list the way
//! a "don't play this one" flag can (ba doc #292, #1397 ruling 3).
//!
//! That works only for a clip the list already holds. Since ba todo #1402 a
//! project load also *loads* a take's recording, and that load is
//! asynchronous: `clips::submit_clip_load` hands the mmap and the peak
//! decimation to a worker, whose finished `AudioClip` reaches the list some
//! milliseconds later. A `RemoveTake` dispatched inside that window used to
//! find nothing to park and park nothing — and the load then published a
//! recording no comp table governs, which plays raw at full gain on the
//! ordinary clip path, on top of the comp. That is #1397's "deleting a take
//! makes it **louder**" returning through a timing window rather than a
//! logic error (measured at peak 1.25 in the engine and 1.5 through the
//! app's real message path).
//!
//! The submit-time early return in
//! [`clips::handle_load_take_clip_from_wav`](crate::engine::clips) cannot
//! close it — it is advisory, and by definition cannot see a load already
//! in flight. The check that *binds* is the one applied with the finished
//! load (`clips::apply_clip_loaded`), so the removal has to agree with
//! **that**: a removal that finds no clip leaves a *claim*, and the load,
//! when it is applied, [`deliver`](TakeClipPark::deliver)s into the park
//! instead of the list.
//!
//! # Engine-thread sequencing (code review ARCH-02 B-5)
//!
//! Until B-5 the load worker published its clip itself, under the clip
//! list's write lock, so this park was shared with the worker and every
//! mutation had to be made while holding the clip list's write lock — that lock
//! is what made "the clip is not in the list" and "the park claims it" one
//! atomic step. The worker now posts its result to the engine thread, and
//! the park, the clip list edit and the load's delivery all run there: a
//! removal (`park_take_clip`) and a load's application
//! (`apply_clip_loaded`) are two engine-thread steps, each applied wholly
//! before or wholly after the other. So the two orderings are still the
//! only two there are — either the removal finds the clip in the list and
//! parks it, or the load finds the claim and delivers into the park — and
//! the park needs no lock, and no lock order, at all.
//!
//! [`AudioClip`]: crate::types::AudioClip

use std::collections::HashMap;
use std::sync::Arc;

use crate::types::{AudioClip, ClipId};

/// What the park knows about one removed take's recording.
enum Parked {
    /// The recording itself, lifted out of the clip list (the very `Arc`
    /// the render graph listed). This is what makes an undo instant:
    /// `RestoreTakeGroups` hands it straight back with no file to
    /// re-open. Dropped only on the engine thread (`ClearAll` clears the
    /// park there), and never the last owner a reader could leave it to:
    /// graphs that still list it are retired and swept on this thread.
    Held(Arc<AudioClip>),
    /// The removal got there first — the clip was not in the list yet
    /// because its `LoadTakeClipFromWav` was still in flight. The claim
    /// stands until something re-claims the take, and the load, when it
    /// lands, is delivered here instead of into the render's input.
    Claimed,
}

/// Recordings of takes that have been removed, held out of the clip list
/// so they cannot sound — plus the ids of removed takes whose
/// recording is still on its way in from a worker.
///
/// Parked rather than dropped, and never deleted from disk: a removal is
/// undoable, and the `RestoreTakeGroups` an undo sends carries the take's
/// `clip_ref` back, so the clip has to still be here for the restored take
/// to be audible as well as visible. Session-local, like the id allocators
/// beside it: `ClearAll` empties it, and a project reload starts from an
/// empty park with the orphaned WAV left on disk.
#[derive(Default)]
pub(crate) struct TakeClipPark {
    entries: HashMap<ClipId, Parked>,
}

#[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
impl TakeClipPark {
    /// Park a recording the clip list *did* hold.
    ///
    /// Overwrites any standing claim for the id, which is the point: the
    /// claim was a promise to capture exactly this clip.
    pub(crate) fn hold(&mut self, clip: Arc<AudioClip>) {
        self.entries.insert(clip.id, Parked::Held(clip));
    }

    /// Claim `clip_id` for the park without a recording to put in it — the
    /// removal beat the load (ba todo #1403).
    ///
    /// Deliberately does **not** overwrite an existing entry. The park is
    /// the only copy of a removed recording that is still in memory, so a
    /// claim displacing one would lose it — and the undo would restore a
    /// take with no audio under it.
    ///
    /// No caller can reach that ordering today: `park_take_clip` skips a
    /// clip any surviving group still claims, and a parked clip is by
    /// definition claimed by none, so the restore's "park what was dropped"
    /// walk never revisits one. This is a guard on the invariant rather
    /// than a live path — stated here because mutating it to a plain
    /// `insert` breaks no test, which is exactly the kind of thing the next
    /// caller would otherwise have to rediscover.
    ///
    /// A claim for an id that no load will ever deliver — a take whose WAV
    /// has gone missing, say — is harmless. It is a claim on the *id*, and
    /// ids are never reused within a session: since D-7d they come from
    /// the app's one clip counter (through the engine's grant), which only
    /// ever rises, and `ClearAll` empties this park anyway.
    pub(crate) fn claim(&mut self, clip_id: ClipId) {
        self.entries
            .entry(clip_id)
            .or_insert(Parked::Claimed);
    }

    /// Give a parked recording back, dropping the park's interest in the id
    /// either way.
    ///
    /// Returns the clip when the park was holding one — push it back into
    /// the clip list and the restored take is audible again, not merely
    /// visible. Returns `None` when the park only had a claim, in which
    /// case dropping the claim is itself the restore: the load that is
    /// still in flight now lands in the clip list as it always would have.
    pub(crate) fn release(&mut self, clip_id: ClipId) -> Option<Arc<AudioClip>> {
        match self.entries.remove(&clip_id) {
            Some(Parked::Held(clip)) => Some(clip),
            Some(Parked::Claimed) | None => None,
        }
    }

    /// Offer a freshly loaded clip to the park.
    ///
    /// Returns `Some(clip)` — hand it back — when the park has no interest
    /// in the id, which is every ordinary load. Returns `None` when the
    /// park took it, i.e. the take it belongs to was removed while this
    /// load was in flight: it is stored rather than dropped, so the undo of
    /// that removal is as instant as any other and never re-reads the WAV.
    ///
    /// Called by `clips::apply_clip_loaded`, in the same engine-thread step
    /// as the duplicate check and the publish.
    pub(crate) fn deliver(&mut self, clip: Arc<AudioClip>) -> Option<Arc<AudioClip>> {
        match self.entries.get(&clip.id) {
            None => Some(clip),
            Some(_) => {
                self.entries.insert(clip.id, Parked::Held(clip));
                None
            }
        }
    }

    /// Drop everything — every parked recording and every standing claim.
    ///
    /// `ClearAll` only: the clip list is drained anyway, so nothing can
    /// un-park into a project that never had these takes, and keeping the
    /// park across a load would hold the previous project's WAV mappings
    /// open.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    /// Clip ids whose recording the park is actually holding, ascending.
    ///
    /// A standing claim is *not* listed: it names a recording that is not
    /// here (yet, or ever), and a test asking "what did the removal park"
    /// wants the audio, not the reservation.
    #[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
    pub(crate) fn held_ids(&self) -> Vec<ClipId> {
        let mut ids: Vec<ClipId> = self
            .entries
            .iter()
            .filter(|(_, parked)| matches!(parked, Parked::Held(_)))
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids
    }
}
