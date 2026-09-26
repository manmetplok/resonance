//! Regression tests for live-note delivery order under plugin-lock
//! contention. A NoteOn that fails `try_lock` used to be reposted to
//! the back of the command queue, so a NoteOff arriving in the
//! meantime could reach the plugin first — leaving a stuck note. The
//! contended event is now parked in a `MidiStash` and always drains
//! ahead of any later direct delivery to the same plugin. Driven
//! through a recording `NoteSink` — no live CLAP plugin needed.

use parking_lot::Mutex;
use resonance_audio::types::PendingNoteEvent;
use resonance_audio::{deliver_or_stash, MidiStash, NoteSink};

#[derive(Debug, PartialEq)]
enum Call {
    On(u8, u32),
    Off(u8, u32),
    AllOff,
}

#[derive(Default)]
struct Recorder {
    calls: Vec<Call>,
}

impl NoteSink for Recorder {
    fn note_on(&mut self, key: u8, _velocity: f32, sample_offset: u32) {
        self.calls.push(Call::On(key, sample_offset));
    }
    fn note_off(&mut self, key: u8, sample_offset: u32) {
        self.calls.push(Call::Off(key, sample_offset));
    }
    fn all_notes_off(&mut self) {
        self.calls.push(Call::AllOff);
    }
}

fn on(note: u8) -> PendingNoteEvent {
    PendingNoteEvent {
        is_note_on: true,
        note,
        velocity: 0.8,
        sample_offset: 0,
    }
}

fn off(note: u8) -> PendingNoteEvent {
    PendingNoteEvent {
        is_note_on: false,
        note,
        velocity: 0.0,
        sample_offset: 0,
    }
}

#[test]
fn contended_note_on_delivers_before_later_note_off() {
    let mutex = Mutex::new(Recorder::default());
    let mut stash = MidiStash::new();

    // NoteOn arrives while the plugin lock is held elsewhere.
    {
        let _held = mutex.lock();
        deliver_or_stash(&mut stash, 7, &mutex, on(60));
    }
    // Lock freed; the matching NoteOff arrives next. The parked NoteOn
    // must drain first or the note sticks.
    deliver_or_stash(&mut stash, 7, &mutex, off(60));

    assert_eq!(
        mutex.lock().calls,
        vec![Call::On(60, 0), Call::Off(60, 0)]
    );
}

#[test]
fn events_parked_across_a_contended_stretch_replay_in_arrival_order() {
    let mutex = Mutex::new(Recorder::default());
    let mut stash = MidiStash::new();

    {
        let _held = mutex.lock();
        deliver_or_stash(&mut stash, 7, &mutex, on(60));
        deliver_or_stash(&mut stash, 7, &mutex, off(60));
        deliver_or_stash(&mut stash, 7, &mutex, on(64));
    }
    deliver_or_stash(&mut stash, 7, &mutex, off(64));

    assert_eq!(
        mutex.lock().calls,
        vec![
            Call::On(60, 0),
            Call::Off(60, 0),
            Call::On(64, 0),
            Call::Off(64, 0),
        ]
    );
}

#[test]
fn uncontended_delivery_bypasses_the_stash() {
    let mutex = Mutex::new(Recorder::default());
    let mut stash = MidiStash::new();

    deliver_or_stash(&mut stash, 7, &mutex, on(60));
    deliver_or_stash(&mut stash, 7, &mutex, off(60));

    assert_eq!(stash.pending_instances().count(), 0);
    assert_eq!(
        mutex.lock().calls,
        vec![Call::On(60, 0), Call::Off(60, 0)]
    );
}

#[test]
fn pending_instances_tracks_parked_plugins_for_the_engine_loop_flush() {
    let mutex = Mutex::new(Recorder::default());
    let mut stash = MidiStash::new();

    {
        let _held = mutex.lock();
        deliver_or_stash(&mut stash, 7, &mutex, on(60));
    }
    assert_eq!(stash.pending_instances().collect::<Vec<_>>(), vec![7]);

    // The engine loop's flush: try_lock succeeded, deliver and free.
    let mut sink = Recorder::default();
    stash.deliver(7, &mut sink);
    assert_eq!(sink.calls, vec![Call::On(60, 0)]);
    assert_eq!(stash.pending_instances().count(), 0);
}
