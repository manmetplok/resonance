//! Regression tests for the plugin-lock-contention MIDI stash: events
//! collected on a block where the instrument's mutex was contended must
//! survive and replay (clamped to offset 0) on the next successful
//! lock, with documented overflow / panic / discard behavior. Driven
//! through a recording `NoteSink` — no live CLAP plugin needed.

use resonance_audio::types::PendingNoteEvent;
use resonance_audio::{
    MidiStash, NoteSink, StashEntry, MAX_STASHED_EVENTS, MAX_STASHED_INSTRUMENTS,
};

#[derive(Debug, PartialEq)]
enum Call {
    On(u8, f32, u32),
    Off(u8, u32),
    AllOff,
    /// A seam panic that keeps the instrument's carried events.
    AllOffKeepCarried,
}

#[derive(Default)]
struct Recorder {
    calls: Vec<Call>,
}

impl NoteSink for Recorder {
    fn note_on(&mut self, key: u8, velocity: f32, sample_offset: u32) {
        self.calls.push(Call::On(key, velocity, sample_offset));
    }
    fn note_off(&mut self, key: u8, sample_offset: u32) {
        self.calls.push(Call::Off(key, sample_offset));
    }
    fn all_notes_off(&mut self) {
        self.calls.push(Call::AllOff);
    }
    fn all_notes_off_keep_carried(&mut self) {
        self.calls.push(Call::AllOffKeepCarried);
    }
}

/// A parked loop-seam panic keeps the note events the instrument carried
/// past the seam — they are the loop's first notes, timed after the panic
/// (FU-A4a). A parked Stop / relocate panic drops them (FU-F2a), and wins
/// when both were parked.
#[test]
fn a_parked_seam_panic_keeps_carried_events_other_panics_drop_them() {
    let mut stash = MidiStash::new();
    stash.request_seam_panic(7);
    let mut sink = Recorder::default();
    stash.deliver(7, &mut sink);
    assert_eq!(sink.calls, vec![Call::AllOffKeepCarried]);

    stash.request_panic(7);
    let mut sink = Recorder::default();
    stash.deliver(7, &mut sink);
    assert_eq!(sink.calls, vec![Call::AllOff]);

    stash.request_panic(7);
    stash.request_seam_panic(7);
    let mut sink = Recorder::default();
    stash.deliver(7, &mut sink);
    assert_eq!(sink.calls, vec![Call::AllOff]);

    // A delivered slot starts clean again.
    stash.request_seam_panic(7);
    let mut sink = Recorder::default();
    stash.deliver(7, &mut sink);
    assert_eq!(sink.calls, vec![Call::AllOffKeepCarried]);
}

fn on(note: u8, sample_offset: u32) -> PendingNoteEvent {
    PendingNoteEvent {
        is_note_on: true,
        note,
        velocity: 0.8,
        sample_offset,
    }
}

fn off(note: u8, sample_offset: u32) -> PendingNoteEvent {
    PendingNoteEvent {
        is_note_on: false,
        note,
        velocity: 0.0,
        sample_offset,
    }
}

#[test]
fn stashed_events_replay_in_order_at_offset_zero() {
    let mut stash = MidiStash::new();
    stash.stash(7, &[off(60, 10), on(60, 10), on(64, 100)]);

    let mut sink = Recorder::default();
    stash.deliver(7, &mut sink);
    assert_eq!(
        sink.calls,
        vec![Call::Off(60, 0), Call::On(60, 0.8, 0), Call::On(64, 0.8, 0)]
    );
}

#[test]
fn delivery_frees_the_slot() {
    let mut stash = MidiStash::new();
    stash.stash(7, &[on(60, 0)]);

    let mut sink = Recorder::default();
    stash.deliver(7, &mut sink);
    sink.calls.clear();
    stash.deliver(7, &mut sink);
    assert!(sink.calls.is_empty());
}

#[test]
fn deliver_for_unknown_instance_is_a_noop() {
    let mut stash = MidiStash::new();
    stash.stash(7, &[on(60, 0)]);

    let mut sink = Recorder::default();
    stash.deliver(8, &mut sink);
    assert!(sink.calls.is_empty());
}

#[test]
fn stash_accumulates_across_contended_blocks() {
    let mut stash = MidiStash::new();
    stash.stash(7, &[on(60, 5)]);
    stash.stash(7, &[off(60, 9)]);

    let mut sink = Recorder::default();
    stash.deliver(7, &mut sink);
    assert_eq!(sink.calls, vec![Call::On(60, 0.8, 0), Call::Off(60, 0)]);
}

#[test]
fn overflow_drops_note_ons_but_keeps_note_offs() {
    let mut stash = MidiStash::new();
    // Fill the slot to capacity with note-ons.
    for _ in 0..MAX_STASHED_EVENTS {
        stash.stash(7, &[on(60, 0)]);
    }
    // A further note-on is dropped; a note-off evicts the oldest
    // note-on to make room.
    stash.stash(7, &[on(61, 0), off(62, 0)]);

    let mut sink = Recorder::default();
    stash.deliver(7, &mut sink);
    assert_eq!(sink.calls.len(), MAX_STASHED_EVENTS);
    assert!(!sink.calls.contains(&Call::On(61, 0.8, 0)));
    assert_eq!(sink.calls.last(), Some(&Call::Off(62, 0)));
    assert_eq!(sink.calls.iter().filter(|c| **c == Call::AllOff).count(), 0);
}

#[test]
fn all_note_off_overflow_degrades_to_all_notes_off() {
    let mut stash = MidiStash::new();
    for _ in 0..MAX_STASHED_EVENTS {
        stash.stash(7, &[off(60, 0)]);
    }
    // No note-on left to evict: the slot degrades to a panic.
    stash.stash(7, &[off(61, 0)]);

    let mut sink = Recorder::default();
    stash.deliver(7, &mut sink);
    assert_eq!(sink.calls.first(), Some(&Call::AllOff));
}

#[test]
fn panic_clears_prior_events_and_precedes_later_ones() {
    let mut stash = MidiStash::new();
    // Pre-seam block stashed under contention…
    stash.stash(7, &[on(60, 5)]);
    // …then the loop-seam panic also hit contention…
    stash.request_panic(7);
    // …then the post-seam block stashed fresh events.
    stash.stash(7, &[on(64, 0)]);

    let mut sink = Recorder::default();
    stash.deliver(7, &mut sink);
    assert_eq!(sink.calls, vec![Call::AllOff, Call::On(64, 0.8, 0)]);
}

#[test]
fn discard_drops_everything_without_delivering() {
    let mut stash = MidiStash::new();
    stash.stash(7, &[on(60, 0)]);
    stash.request_panic(7);
    stash.discard(7);

    let mut sink = Recorder::default();
    stash.deliver(7, &mut sink);
    assert!(sink.calls.is_empty());
}

#[test]
fn slot_pool_exhaustion_drops_events_for_new_instances() {
    let mut stash = MidiStash::new();
    for id in 0..MAX_STASHED_INSTRUMENTS as u64 {
        stash.stash(id, &[on(60, 0)]);
    }
    // Pool exhausted: a 65th contended instrument loses its block…
    stash.stash(999, &[on(61, 0)]);
    let mut sink = Recorder::default();
    stash.deliver(999, &mut sink);
    assert!(sink.calls.is_empty());

    // …but existing slots still deliver, and freeing one readmits
    // the newcomer.
    stash.deliver(0, &mut sink);
    assert_eq!(sink.calls, vec![Call::On(60, 0.8, 0)]);
    stash.stash(999, &[on(61, 0)]);
    sink.calls.clear();
    stash.deliver(999, &mut sink);
    assert_eq!(sink.calls, vec![Call::On(61, 0.8, 0)]);
}

fn ev(note: u8, on: bool) -> PendingNoteEvent {
    PendingNoteEvent {
        is_note_on: on,
        note,
        velocity: 0.5,
        sample_offset: 17,
    }
}

/// A render job borrows an instrument's parked events by value
/// (`take` → the job's carry → `restore`) so it can run on another
/// thread without sharing the stash (realtime-multithreading.md §4.2).
/// A carry the job never touched must come back exactly as it left —
/// events, order and a pending panic all intact.
#[test]
fn an_untouched_carry_round_trips_unchanged() {
    let mut stash = MidiStash::new();
    stash.request_panic(3);
    stash.stash(3, &[ev(60, true), ev(62, false)]);
    let mut carry = StashEntry::new();

    assert!(stash.take(3, &mut carry));
    assert!(!stash.has_pending(), "taking frees the stash slot");
    assert_eq!(carry.instance(), Some(3));
    stash.restore(&mut carry);
    assert_eq!(carry.instance(), None, "restoring frees the carry");

    let mut sink = Recorder::default();
    stash.deliver(3, &mut sink);
    assert_eq!(
        sink.calls,
        vec![Call::AllOff, Call::On(60, 0.5, 0), Call::Off(62, 0)]
    );
}

/// A job whose lock failed again parks its block's events into the carry
/// after the ones already there — the same order `stash` alone keeps.
#[test]
fn a_carry_stashed_into_restores_old_events_then_new() {
    let mut stash = MidiStash::new();
    stash.stash(3, &[ev(60, true)]);
    let mut carry = StashEntry::new();
    stash.take(3, &mut carry);
    carry.stash(3, &[ev(60, false), ev(64, true)]);
    stash.restore(&mut carry);

    let mut sink = Recorder::default();
    stash.deliver(3, &mut sink);
    assert_eq!(
        sink.calls,
        vec![Call::On(60, 0.5, 0), Call::Off(60, 0), Call::On(64, 0.5, 0)]
    );
}

/// A job whose lock succeeded delivers from the carry, which leaves
/// nothing to restore.
#[test]
fn a_delivered_carry_leaves_nothing_parked() {
    let mut stash = MidiStash::new();
    stash.stash(3, &[ev(60, true)]);
    let mut carry = StashEntry::new();
    stash.take(3, &mut carry);

    let mut sink = Recorder::default();
    carry.deliver(4, &mut sink);
    assert!(
        sink.calls.is_empty(),
        "another instrument's carry is not delivered"
    );
    carry.deliver(3, &mut sink);
    assert_eq!(sink.calls, vec![Call::On(60, 0.5, 0)]);

    stash.restore(&mut carry);
    assert!(!stash.has_pending());
}

/// With every stash slot claimed by other instruments while the carry
/// was out, restoring drops the carry — the same fate `stash` gives a
/// block's events on an exhausted pool — rather than panicking.
#[test]
fn restoring_into_an_exhausted_pool_drops_the_carry() {
    let mut stash = MidiStash::new();
    stash.stash(1000, &[ev(60, true)]);
    let mut carry = StashEntry::new();
    stash.take(1000, &mut carry);
    for id in 0..MAX_STASHED_INSTRUMENTS as u64 {
        stash.stash(id, &[ev(1, true)]);
    }
    stash.restore(&mut carry);
    assert_eq!(carry.instance(), None);
    assert!(!stash.pending_instances().any(|id| id == 1000));
}
