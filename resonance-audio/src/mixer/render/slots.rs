//! Per-track render slots: where a track job leaves its result for the
//! ordered reduction (realtime-multithreading.md §3, §4.1).
//!
//! The track pass is split at the summing point. A **job** renders one
//! top-level track — source, insert chain, key capture, PDC, meters and
//! its multi-output fan-out — into the [`TrackSlot`]s it owns: its own,
//! at the track's index in the track map, and one per sub-track it feeds,
//! at the sub-track's index. It never touches a shared sum. The
//! **reduction** then walks the tracks in map order and replays exactly
//! the additions the old single loop made (post-fader route, aux sends,
//! sub-track routes), so the mix is bit-identical however the jobs were
//! scheduled.
//!
//! Slots are indexed by track-map position, so a job's set is disjoint
//! from every other job's by construction: a top-level track owns its own
//! index, and a sub-track's index belongs to the one parent its
//! `sub_track_of` names.
//!
//! # Sizing
//!
//! The pool must hold one slot per track, and the audio thread cannot
//! allocate. Offline renderers (bounce, tests) size it themselves with
//! [`TrackSlots::ensure`]. The live callback's pool is grown on the
//! engine thread instead: [`SlotSupply::ensure`] runs on every render-graph
//! publish, *before* the new graph is stored, and hands a larger pool over
//! a bounded channel that [`LiveSlots::refresh`] picks up at the top of the
//! next callback. The replaced pool travels back the same way and is
//! dropped on the engine thread.

use std::marker::PhantomData;

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;

use crate::mixer::midi_stash::StashEntry;
use crate::types::{TrackId, TrackOutput};

use super::context::GainRamp;

/// Smallest pool the live callback starts with, so a typical project
/// never needs a hand-over at all.
const MIN_LIVE_SLOTS: usize = 64;

/// A job's decision that its track (or sub-track) reaches the mix this
/// block, and how.
#[derive(Clone, Copy)]
pub(crate) struct SlotRoute {
    /// `None` forces the master route (freeze capture); see
    /// `route_post_fader`.
    pub(crate) dest: Option<TrackOutput>,
    pub(crate) gains: GainRamp,
}

/// One track's (or sub-track's) per-block output.
pub(crate) struct TrackSlot {
    /// The post-FX, post-PDC, pre-fader signal. `max_frames` long; a block
    /// uses the first `frames`.
    pub(crate) l: Vec<f32>,
    pub(crate) r: Vec<f32>,
    /// Set by the job when the signal is to be summed; taken (and so
    /// cleared) by the reduction.
    pub(crate) route: Option<SlotRoute>,
    /// The job ran this track's sub-track fan-out, so the reduction must
    /// visit its sub-tracks' slots.
    pub(crate) fanned_out: bool,
    /// Whether a sidechain key is tapped from this track by a consumer
    /// that reads it (`key_consumed`), decided once per block before any
    /// job runs. The decision reads other tracks' live state (their last
    /// gains and bypass fades), which their own jobs update, so it must
    /// not depend on which job happened to run first.
    pub(crate) key_consumed: bool,
    /// MIDI parked for this track's instrument during earlier lock
    /// contention, lent by the `MidiStash` for the block (live only).
    pub(crate) carry: StashEntry,
    /// Nanoseconds this track's last job took (its own render plus its
    /// fan-out), and an EMA of it — the schedule's cost estimate.
    pub(crate) last_ns: u32,
    pub(crate) cost_ns: u32,
}

impl TrackSlot {
    fn new(frames: usize) -> Self {
        let mut l = vec![0.0f32; frames];
        let mut r = vec![0.0f32; frames];
        crate::prefault::prefault_f32(&mut l);
        crate::prefault::prefault_f32(&mut r);
        Self {
            l,
            r,
            route: None,
            fanned_out: false,
            key_consumed: false,
            carry: StashEntry::new(),
            last_ns: 0,
            cost_ns: 0,
        }
    }
}

/// The slot pool one renderer owns. See the module docs.
pub(crate) struct TrackSlots {
    slots: Vec<TrackSlot>,
    /// The block's job list: slot indices of the top-level tracks, in
    /// claim order. Capacity for every slot, so building it never
    /// allocates.
    order: Vec<u32>,
    frames: usize,
    /// What the track passes since the last [`Self::take_stats`] measured.
    pub(crate) stats: PassStats,
}

/// Track-pass measurements for the load report
/// (realtime-multithreading.md §6), accumulated over a callback's
/// sub-blocks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassStats {
    /// Summed job time across all threads.
    pub jobs_ns: u64,
    /// Wall time of the job phase (dispatch to join).
    pub wall_ns: u64,
    /// The longest single job, and whose it was.
    pub critical_ns: u64,
    pub critical_track: Option<TrackId>,
    /// Time the rendering thread spun at the join with nothing to claim.
    pub join_wait_ns: u64,
    /// Threads the jobs were spread over (caller included).
    pub threads: usize,
}

impl PassStats {
    fn merge(&mut self, other: PassStats) {
        self.jobs_ns += other.jobs_ns;
        self.wall_ns += other.wall_ns;
        if other.critical_ns > self.critical_ns {
            self.critical_ns = other.critical_ns;
            self.critical_track = other.critical_track;
        }
        self.join_wait_ns += other.join_wait_ns;
        self.threads = self.threads.max(other.threads);
    }
}

impl TrackSlots {
    /// A pool of `capacity` slots, each `frames` long. Allocates.
    pub(crate) fn new(capacity: usize, frames: usize) -> Self {
        let frames = frames.max(1);
        Self {
            slots: (0..capacity).map(|_| TrackSlot::new(frames)).collect(),
            order: Vec::with_capacity(capacity),
            frames,
            stats: PassStats::default(),
        }
    }

    /// Grow to at least `tracks` slots. Allocates; never call it on the
    /// audio thread.
    pub(crate) fn ensure(&mut self, tracks: usize) {
        while self.slots.len() < tracks {
            self.slots.push(TrackSlot::new(self.frames));
        }
        self.order.reserve(self.slots.len().saturating_sub(self.order.len()));
    }

    /// The slots and the job-order buffer, borrowed apart.
    pub(crate) fn parts(&mut self) -> (&mut [TrackSlot], &mut Vec<u32>) {
        (&mut self.slots, &mut self.order)
    }

    pub(crate) fn record(&mut self, stats: PassStats) {
        self.stats.merge(stats);
    }

    /// The stats accumulated since the last call.
    pub(crate) fn take_stats(&mut self) -> PassStats {
        std::mem::take(&mut self.stats)
    }
}

/// The slots of one block's jobs, shared by every thread running them.
///
/// Jobs need `&mut` access to the slots they own while other threads hold
/// other slots, and which slots a job owns is decided by the track graph,
/// not by a split the borrow checker can see. This is that split, taken on
/// trust from the ownership rule in the module docs: a track job touches
/// only its own track's slot and its sub-tracks' slots. The bus pass uses
/// the same view over its buffers, where a bus job owns its own bus and
/// only reads busses finished at an earlier level.
pub(crate) struct SlotCells<'a, T = TrackSlot> {
    ptr: *mut T,
    len: usize,
    _slots: PhantomData<&'a mut [T]>,
}

// SAFETY: the cells hand an element to one job at a time (see `get`),
// and only ever to threads the pool joins before the borrow ends.
unsafe impl<T: Send> Sync for SlotCells<'_, T> {}

impl<'a, T> SlotCells<'a, T> {
    pub(crate) fn new(slots: &'a mut [T]) -> Self {
        Self {
            ptr: slots.as_mut_ptr(),
            len: slots.len(),
            _slots: PhantomData,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Element `idx`, mutably.
    ///
    /// # Safety
    ///
    /// The calling job must own `idx` — for the track pass, its own
    /// top-level track or one of that track's sub-tracks; for the bus
    /// pass, its own bus — and must not hold another reference to the
    /// same element while this one lives.
    #[allow(clippy::mut_from_ref)]
    pub(crate) unsafe fn get(&self, idx: usize) -> &mut T {
        assert!(idx < self.len, "slot {idx} out of {}", self.len);
        // SAFETY: in bounds (checked); exclusive per the contract.
        unsafe { &mut *self.ptr.add(idx) }
    }

    /// Element `idx`, shared.
    ///
    /// # Safety
    ///
    /// Nothing may write `idx` while this lives: it belongs to a job that
    /// finished before the current run started.
    pub(crate) unsafe fn get_ref(&self, idx: usize) -> &T {
        assert!(idx < self.len, "slot {idx} out of {}", self.len);
        // SAFETY: in bounds (checked); read-only per the contract.
        unsafe { &*self.ptr.add(idx) }
    }
}

/// The audio thread's end of the live slot pool: the pool it renders
/// with, and the channels a larger one arrives on and the old one leaves
/// by. See the module docs.
pub(crate) struct LiveSlots {
    current: Box<TrackSlots>,
    offer_rx: Receiver<Box<TrackSlots>>,
    return_tx: Sender<Box<TrackSlots>>,
}

impl LiveSlots {
    /// Adopt the newest pool the engine offered, if any, sending the
    /// replaced one back to be dropped on the engine thread. Audio thread;
    /// allocation-free and non-blocking.
    pub(crate) fn refresh(&mut self) {
        while let Ok(next) = self.offer_rx.try_recv() {
            let old = std::mem::replace(&mut self.current, next);
            if let Err(err) = self.return_tx.try_send(old) {
                // The return queue is drained on every publish and holds
                // far more than one growth's worth, so this cannot
                // happen; if it ever did, leaking one pool is the only
                // choice that doesn't free memory on the audio thread.
                std::mem::forget(err.into_inner());
            }
        }
    }

    pub(crate) fn current(&mut self) -> &mut TrackSlots {
        &mut self.current
    }
}

/// The engine thread's end of the live slot pool. Lives on the
/// `RenderGraphSlot`, whose every publish calls [`Self::ensure`].
#[derive(Default)]
pub(crate) struct SlotSupply {
    link: Mutex<Option<SupplyLink>>,
}

struct SupplyLink {
    frames: usize,
    /// Slots in the pool the audio thread has, or has been offered.
    capacity: usize,
    offer_tx: Sender<Box<TrackSlots>>,
    /// Kept so an offer the audio thread has not collected yet can be
    /// taken back and replaced by a larger one.
    offer_rx: Receiver<Box<TrackSlots>>,
    return_rx: Receiver<Box<TrackSlots>>,
}

impl std::fmt::Debug for SlotSupply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let link = self.link.lock();
        f.debug_struct("SlotSupply")
            .field("capacity", &link.as_ref().map(|l| l.capacity))
            .finish()
    }
}

impl SlotSupply {
    /// Start supplying a new live callback whose blocks are at most
    /// `frames` long, with a pool that already fits `tracks`. Replaces
    /// any previous link (an output stream rebuilt after a device
    /// change). Allocates; engine side.
    pub(crate) fn attach(&self, frames: usize, tracks: usize) -> LiveSlots {
        let capacity = tracks.next_power_of_two().max(MIN_LIVE_SLOTS);
        let (offer_tx, offer_rx) = crossbeam_channel::bounded(1);
        let (return_tx, return_rx) = crossbeam_channel::bounded(8);
        let live = LiveSlots {
            current: Box::new(TrackSlots::new(capacity, frames)),
            offer_rx: offer_rx.clone(),
            return_tx,
        };
        *self.link.lock() = Some(SupplyLink {
            frames,
            capacity,
            offer_tx,
            offer_rx,
            return_rx,
        });
        live
    }

    /// Make sure the live callback will have a slot for each of `tracks`
    /// by the time it can see a graph that has them. Called with the
    /// graph's edit lock held, before the new graph is stored: the offer
    /// is sent before the store, and the callback loads the graph before
    /// it polls the offer, so a callback that sees the graph sees the
    /// pool. Also drops pools the callback has handed back. Engine
    /// thread; allocates only when growing.
    pub(crate) fn ensure(&self, tracks: usize) {
        let mut guard = self.link.lock();
        let Some(link) = guard.as_mut() else {
            return;
        };
        while link.return_rx.try_recv().is_ok() {}
        if tracks <= link.capacity {
            return;
        }
        let capacity = tracks.next_power_of_two().max(link.capacity * 2);
        // An uncollected smaller offer is superseded.
        while link.offer_rx.try_recv().is_ok() {}
        let pool = Box::new(TrackSlots::new(capacity, link.frames));
        if link.offer_tx.try_send(pool).is_ok() {
            link.capacity = capacity;
        }
    }
}
