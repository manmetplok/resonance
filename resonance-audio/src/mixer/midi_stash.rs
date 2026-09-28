//! Lock-contention MIDI stash: when the audio thread's `try_lock` on an
//! instrument plugin fails (UI thread holds it for a param drag,
//! autosave, or reload), the block's collected note events are parked
//! here instead of being dropped, then replayed at sample offset 0 on
//! the next block where the lock succeeds — no stuck or missing notes.
//!
//! Fixed capacity, allocation-free after construction (audio-thread
//! safe; each stash instance is owned by a single thread — the cpal
//! callback closure owns the mixer's, and the engine control thread
//! owns a second instance for live note input). Overflow behavior:
//! - Slot buffer full + incoming note-on: the note-on is dropped.
//! - Slot buffer full + incoming note-off: the oldest stashed note-on
//!   is evicted to make room; if none exists, the slot degrades to a
//!   panic (all-notes-off on the next successful lock).
//! - Slot pool exhausted (`MAX_STASHED_INSTRUMENTS` simultaneously
//!   contended instruments): the block's events are dropped.
//!
//! The matching one-block audio dropout is still accepted; future work
//! could crossfade the re-locked block in.

use crate::clap_host::SyncClapInstance;
use crate::limits::{MAX_STASHED_EVENTS, MAX_STASHED_INSTRUMENTS};
use crate::types::{PendingNoteEvent, PluginInstanceId};

/// Receiver for replayed stash events. `SyncClapInstance` is the
/// production sink; tests substitute a recorder.
pub trait NoteSink {
    fn note_on(&mut self, key: u8, velocity: f32, sample_offset: u32);
    fn note_off(&mut self, key: u8, sample_offset: u32);
    fn all_notes_off(&mut self);
    /// A parked loop-seam panic: like [`Self::all_notes_off`], but the
    /// events an instrument carried past the seam's head sub-block are
    /// timed after the panic and survive it (FU-A4a). Defaults to the
    /// plain panic for sinks with no carried events.
    fn all_notes_off_keep_carried(&mut self) {
        self.all_notes_off();
    }
}

impl NoteSink for SyncClapInstance {
    // Live notes (preview, controller, their stash replays) keep the
    // instrument processed while stopped (code review MIX-08).
    fn note_on(&mut self, key: u8, velocity: f32, sample_offset: u32) {
        self.0.arm_idle_hold();
        self.0.queue_note_on(key, velocity, sample_offset);
    }
    fn note_off(&mut self, key: u8, sample_offset: u32) {
        self.0.arm_idle_hold();
        self.0.queue_note_off(key, sample_offset);
    }
    // A parked Stop / relocate panic has no "after" for carried events
    // to belong to — drop them (FU-F2a).
    fn all_notes_off(&mut self) {
        self.0.all_notes_off_and_drop_carried();
    }
    // A parked seam panic: carried events are the loop's first notes,
    // timed after it — keep them (FU-A4a).
    fn all_notes_off_keep_carried(&mut self) {
        self.0.all_notes_off();
    }
}

/// One instrument's parked events. A [`MidiStash`] holds a fixed pool of
/// these; a render job borrows one by value through
/// [`MidiStash::take`] / [`MidiStash::restore`] (a `Vec` swap, so neither
/// direction allocates), which is what lets the stash stay single-owner
/// while track jobs run on other threads (realtime-multithreading.md
/// §4.2).
pub struct StashEntry {
    instance: Option<PluginInstanceId>,
    events: Vec<PendingNoteEvent>,
    /// Deliver an all-notes-off before any stashed events on the next
    /// successful lock. Set by `request_panic` (a panic that couldn't
    /// take the plugin lock) and by note-off overflow.
    panic: bool,
    /// The parked panic must also drop the instrument's carried events:
    /// set by every panic except a loop seam's (FU-A4a).
    drop_carried: bool,
}

impl StashEntry {
    /// An empty entry with the full event capacity pre-allocated.
    pub fn new() -> Self {
        Self {
            instance: None,
            events: Vec::with_capacity(MAX_STASHED_EVENTS),
            panic: false,
            drop_carried: false,
        }
    }

    /// The instrument this entry holds events for, if any.
    pub fn instance(&self) -> Option<PluginInstanceId> {
        self.instance
    }

    fn clear(&mut self) {
        self.instance = None;
        self.events.clear();
        self.panic = false;
        self.drop_carried = false;
    }

    /// Park a contended block's events for `id`. The entry must be free
    /// or already hold `id`; the overflow rules are the module docs'.
    pub fn stash(&mut self, id: PluginInstanceId, events: &[PendingNoteEvent]) {
        if events.is_empty() {
            return;
        }
        self.instance = Some(id);
        for event in events {
            if self.events.len() < MAX_STASHED_EVENTS {
                self.events.push(event.clone());
                continue;
            }
            if event.is_note_on {
                // Overflow: note-ons are droppable.
                continue;
            }
            // Overflow with a note-off: evict the oldest stashed note-on
            // to make room; if every stashed event is a note-off, degrade
            // to a panic — all-notes-off supersedes them all.
            if let Some(idx) = self.events.iter().position(|e| e.is_note_on) {
                self.events.remove(idx);
                self.events.push(event.clone());
            } else {
                self.events.clear();
                self.panic = true;
                self.drop_carried = true;
            }
        }
    }

    /// Replay everything parked here into `sink` if it belongs to `id`,
    /// and free the entry. See [`MidiStash::deliver`].
    pub fn deliver(&mut self, id: PluginInstanceId, sink: &mut impl NoteSink) {
        if self.instance != Some(id) {
            return;
        }
        if self.panic {
            if self.drop_carried {
                sink.all_notes_off();
            } else {
                sink.all_notes_off_keep_carried();
            }
        }
        for event in &self.events {
            if event.is_note_on {
                sink.note_on(event.note, event.velocity, 0);
            } else {
                sink.note_off(event.note, 0);
            }
        }
        self.clear();
    }

    /// Move this entry's contents into `other` (which must be free),
    /// leaving this one free. Swaps the event buffers, so both keep their
    /// pre-allocated capacity and nothing allocates.
    fn move_into(&mut self, other: &mut StashEntry) {
        std::mem::swap(&mut self.events, &mut other.events);
        other.instance = self.instance.take();
        other.panic = std::mem::take(&mut self.panic);
        other.drop_carried = std::mem::take(&mut self.drop_carried);
        self.events.clear();
    }
}

pub struct MidiStash {
    slots: Vec<StashEntry>,
}

impl MidiStash {
    pub fn new() -> Self {
        Self {
            slots: (0..MAX_STASHED_INSTRUMENTS)
                .map(|_| StashEntry::new())
                .collect(),
        }
    }

    /// Find the slot already holding `id`, or claim a free one.
    fn slot_mut(&mut self, id: PluginInstanceId) -> Option<&mut StashEntry> {
        let idx = self
            .slots
            .iter()
            .position(|s| s.instance == Some(id))
            .or_else(|| self.slots.iter().position(|s| s.instance.is_none()))?;
        let slot = &mut self.slots[idx];
        slot.instance = Some(id);
        Some(slot)
    }

    /// Park a contended block's events for `id`.
    pub fn stash(&mut self, id: PluginInstanceId, events: &[PendingNoteEvent]) {
        if events.is_empty() {
            return;
        }
        if let Some(slot) = self.slot_mut(id) {
            slot.stash(id, events);
        }
    }

    /// Whether anything at all is parked — the render pass's fast path
    /// for skipping [`Self::take`].
    pub fn has_pending(&self) -> bool {
        self.slots.iter().any(|s| s.instance.is_some())
    }

    /// Move whatever is parked for `id` into `carry` (which must be
    /// free), freeing the stash slot. The render job that owns `id`'s
    /// instrument delivers from `carry` on a successful lock, or parks
    /// more into it on a failed one; [`Self::restore`] puts back whatever
    /// is left. Returns whether anything was moved. Allocation-free.
    pub fn take(&mut self, id: PluginInstanceId, carry: &mut StashEntry) -> bool {
        match self.slots.iter_mut().find(|s| s.instance == Some(id)) {
            Some(slot) => {
                slot.move_into(carry);
                true
            }
            None => false,
        }
    }

    /// Return a render job's `carry` to the stash, merging it into any
    /// slot parked for the same instrument since, and leave `carry` free.
    /// A free `carry` is a no-op. With the slot pool exhausted the carry
    /// is dropped, exactly as [`Self::stash`] drops a block's events.
    pub fn restore(&mut self, carry: &mut StashEntry) {
        let Some(id) = carry.instance else {
            return;
        };
        match self.slots.iter().position(|s| s.instance == Some(id)) {
            // Merge (unreachable while a carry is out: nothing else parks
            // for an instrument its job owns — kept for robustness).
            Some(idx) => {
                let slot = &mut self.slots[idx];
                if carry.panic {
                    slot.events.clear();
                    slot.panic = true;
                    slot.drop_carried |= carry.drop_carried;
                }
                let events = std::mem::take(&mut carry.events);
                slot.stash(id, &events);
                carry.events = events;
                carry.clear();
            }
            None => match self.slots.iter_mut().find(|s| s.instance.is_none()) {
                Some(free) => carry.move_into(free),
                None => carry.clear(),
            },
        }
    }

    /// Instances that currently hold parked events or a pending panic.
    /// Used by the engine thread's live-note flush to retry delivery
    /// even when no further input arrives for a contended plugin.
    pub fn pending_instances(&self) -> impl Iterator<Item = PluginInstanceId> + '_ {
        self.slots.iter().filter_map(|s| s.instance)
    }

    /// Drop everything parked for `id` without delivering. Used when an
    /// all-notes-off reached the plugin directly, superseding any
    /// stashed pre-panic events.
    pub fn discard(&mut self, id: PluginInstanceId) {
        if let Some(slot) = self.slots.iter_mut().find(|s| s.instance == Some(id)) {
            slot.clear();
        }
    }

    /// Request an all-notes-off on the next successful lock for `id`
    /// (used when a Stop / relocate panic couldn't take the plugin
    /// lock). Clears any stashed events — they predate the panic — and
    /// drops the instrument's carried events on delivery (FU-F2a).
    pub fn request_panic(&mut self, id: PluginInstanceId) {
        if let Some(slot) = self.slot_mut(id) {
            slot.events.clear();
            slot.panic = true;
            slot.drop_carried = true;
        }
    }

    /// [`Self::request_panic`] for a loop-seam panic: the events the
    /// instrument carried past the seam's head sub-block are timed after
    /// the seam, so delivery keeps them (FU-A4a) — unless another parked
    /// panic already asked for them to go.
    pub fn request_seam_panic(&mut self, id: PluginInstanceId) {
        if let Some(slot) = self.slot_mut(id) {
            slot.events.clear();
            slot.panic = true;
        }
    }

    /// Replay everything parked for `id` into `sink` and free the slot.
    /// Call immediately after a successful lock, before queueing the
    /// current block's events. Stashed offsets refer to a past block, so
    /// they're clamped to 0 (the start of the current block); insertion
    /// order keeps note-offs ahead of retriggered note-ons.
    pub fn deliver(&mut self, id: PluginInstanceId, sink: &mut impl NoteSink) {
        if let Some(slot) = self.slots.iter_mut().find(|s| s.instance == Some(id)) {
            slot.deliver(id, sink);
        }
    }
}

impl Default for MidiStash {
    fn default() -> Self {
        Self::new()
    }
}
