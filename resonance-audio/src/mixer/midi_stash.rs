//! Lock-contention MIDI stash: when the audio thread's `try_lock` on an
//! instrument plugin fails (UI thread holds it for a param drag,
//! autosave, or reload), the block's collected note events are parked
//! here instead of being dropped, then replayed at sample offset 0 on
//! the next block where the lock succeeds — no stuck or missing notes.
//!
//! Fixed capacity, allocation-free after construction (audio-thread
//! safe; the stash is owned by the cpal callback closure and never
//! touched by other threads). Overflow behavior:
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
}

impl NoteSink for SyncClapInstance {
    fn note_on(&mut self, key: u8, velocity: f32, sample_offset: u32) {
        self.0.queue_note_on(key, velocity, sample_offset);
    }
    fn note_off(&mut self, key: u8, sample_offset: u32) {
        self.0.queue_note_off(key, sample_offset);
    }
    fn all_notes_off(&mut self) {
        self.0.all_notes_off();
    }
}

struct Slot {
    instance: Option<PluginInstanceId>,
    events: Vec<PendingNoteEvent>,
    /// Deliver an all-notes-off before any stashed events on the next
    /// successful lock. Set by `request_panic` (loop-seam panic that
    /// couldn't take the lock) and by note-off overflow.
    panic: bool,
}

pub struct MidiStash {
    slots: Vec<Slot>,
}

impl MidiStash {
    pub fn new() -> Self {
        Self {
            slots: (0..MAX_STASHED_INSTRUMENTS)
                .map(|_| Slot {
                    instance: None,
                    events: Vec::with_capacity(MAX_STASHED_EVENTS),
                    panic: false,
                })
                .collect(),
        }
    }

    /// Find the slot already holding `id`, or claim a free one.
    fn slot_mut(&mut self, id: PluginInstanceId) -> Option<&mut Slot> {
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
        let Some(slot) = self.slot_mut(id) else {
            return;
        };
        for event in events {
            if slot.events.len() < MAX_STASHED_EVENTS {
                slot.events.push(event.clone());
                continue;
            }
            if event.is_note_on {
                // Overflow: note-ons are droppable.
                continue;
            }
            // Overflow with a note-off: evict the oldest stashed note-on
            // to make room; if every stashed event is a note-off, degrade
            // to a panic — all-notes-off supersedes them all.
            if let Some(idx) = slot.events.iter().position(|e| e.is_note_on) {
                slot.events.remove(idx);
                slot.events.push(event.clone());
            } else {
                slot.events.clear();
                slot.panic = true;
            }
        }
    }

    /// Drop everything parked for `id` without delivering. Used when an
    /// all-notes-off reached the plugin directly, superseding any
    /// stashed pre-panic events.
    pub fn discard(&mut self, id: PluginInstanceId) {
        if let Some(slot) = self.slots.iter_mut().find(|s| s.instance == Some(id)) {
            slot.instance = None;
            slot.events.clear();
            slot.panic = false;
        }
    }

    /// Request an all-notes-off on the next successful lock for `id`
    /// (used when the loop-seam panic couldn't take the plugin lock).
    /// Clears any stashed events — they predate the panic.
    pub fn request_panic(&mut self, id: PluginInstanceId) {
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
        let Some(slot) = self.slots.iter_mut().find(|s| s.instance == Some(id)) else {
            return;
        };
        if slot.panic {
            sink.all_notes_off();
        }
        for event in &slot.events {
            if event.is_note_on {
                sink.note_on(event.note, event.velocity, 0);
            } else {
                sink.note_off(event.note, 0);
            }
        }
        slot.instance = None;
        slot.events.clear();
        slot.panic = false;
    }
}

impl Default for MidiStash {
    fn default() -> Self {
        Self::new()
    }
}
