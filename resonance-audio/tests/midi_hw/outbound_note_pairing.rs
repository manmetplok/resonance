//! Integration tests for the on/off pairing of the timeline → hardware
//! MIDI scheduler (`engine/midi/outbound.rs`).
//!
//! A hardware synth tracks voices per key, so the wire has to stay
//! strictly 1:1 NoteOn/NoteOff per pitch. Two NoteOns for one key with a
//! single NoteOff between them leaves a voice sounding until the next
//! panic — the "hanging note on the external synth" symptom. That is easy
//! to hit because the emitter works one ~16 ms poll window at a time:
//! a note ending exactly where the next note of the same pitch starts
//! puts both messages in the same window, as does any repeat whose gap is
//! shorter than the poll period.
//!
//! The tests drive the pure emission core `emit_outbound_notes` with a
//! capturing fake `OutboundNoteSink` — the same core the engine poll and
//! the realtime bounce drive run.

use std::collections::HashMap;

use resonance_audio::types::*;
use resonance_audio::{emit_outbound_notes, OutboundNoteSink, OutboundTrack};

const SR: u32 = 48_000;
const TRACK: TrackId = 1;

/// At the default tempo map (120 BPM, 480 TPQN, 48 kHz) one tick is
/// exactly 50 samples, so tick positions map to round sample positions.
const SAMPLES_PER_TICK: u64 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Msg {
    On(TrackId, u8, u8, u8),
    Off(TrackId, u8, u8),
}

#[derive(Default)]
struct CaptureSink(Vec<Msg>);

impl OutboundNoteSink for CaptureSink {
    fn note_on(&mut self, track_id: TrackId, channel: u8, note: u8, velocity: u8) {
        self.0.push(Msg::On(track_id, channel, note, velocity));
    }
    fn note_off(&mut self, track_id: TrackId, channel: u8, note: u8) {
        self.0.push(Msg::Off(track_id, channel, note));
    }
}

fn midi_clip(notes: Vec<MidiNote>) -> MidiClip {
    MidiClip {
        id: 1,
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 4_000,
        notes,
        name: "part".into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

fn note(pitch: u8, start_tick: u64, duration_ticks: u64) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: 1.0,
        start_tick,
        duration_ticks,
    }
}

fn track_entry() -> OutboundTrack {
    OutboundTrack {
        track_id: TRACK,
        channel: 0,
        gate_recorded: false,
    }
}

/// Run the emitter over contiguous poll windows of `step` samples across
/// `[0, end)`, mimicking the engine poll cadence, and return the emitted
/// message stream.
fn run_windows(midi_clips: &[MidiClip], end: u64, step: u64) -> Vec<Msg> {
    let tempo = TempoMap::default();
    let tracks = [track_entry()];
    let mut held: HashMap<(TrackId, u8), (u64, u8)> = HashMap::new();
    let mut sink = CaptureSink::default();
    let mut last = 0u64;
    while last < end {
        let curr = (last + step).min(end);
        emit_outbound_notes(
            &tracks, midi_clips, &[] as &[AudioClip], &tempo, SR, last, curr, &mut held, &mut sink,
        );
        last = curr;
    }
    sink.0
}

/// Every NoteOn for a pitch is answered by a NoteOff before the next
/// NoteOn for that same pitch, and nothing is left sounding at the end.
/// This is the property a hardware synth's voice allocator depends on.
fn assert_paired(msgs: &[Msg]) {
    let mut sounding: HashMap<(TrackId, u8, u8), usize> = HashMap::new();
    for msg in msgs {
        match *msg {
            Msg::On(t, ch, n, _) => {
                let count = sounding.entry((t, ch, n)).or_default();
                assert_eq!(
                    *count, 0,
                    "second NoteOn for {n} with no NoteOff between them: {msgs:?}"
                );
                *count += 1;
            }
            Msg::Off(t, ch, n) => {
                let count = sounding.entry((t, ch, n)).or_default();
                assert_eq!(*count, 1, "NoteOff for {n} that is not sounding: {msgs:?}");
                *count -= 1;
            }
        }
    }
    assert!(
        sounding.values().all(|c| *c == 0),
        "notes left sounding at the end: {msgs:?}"
    );
}

/// Back-to-back repeats of one pitch — note N ends exactly where note N+1
/// starts. Both messages land in the same poll window, and the NoteOff
/// must be emitted first.
#[test]
fn adjacent_repeats_of_one_pitch_release_before_retrigger() {
    // Ticks 0..40, 40..80, 80..120 → samples 0..2_000, 2_000..4_000, ...
    let clips = vec![midi_clip(vec![
        note(60, 0, 40),
        note(60, 40, 40),
        note(60, 80, 40),
    ])];

    // A poll step wider than the note length puts each Off and the next
    // On in the same window — the engine's ~16 ms cadence against notes
    // of a few hundred samples.
    let msgs = run_windows(&clips, 8_000, 3_000);

    assert_paired(&msgs);
    assert_eq!(
        msgs,
        vec![
            Msg::On(TRACK, 0, 60, 127),
            Msg::Off(TRACK, 0, 60),
            Msg::On(TRACK, 0, 60, 127),
            Msg::Off(TRACK, 0, 60),
            Msg::On(TRACK, 0, 60, 127),
            Msg::Off(TRACK, 0, 60),
        ]
    );
}

/// The same pattern with a poll step narrow enough that each message gets
/// its own window still produces the identical stream — the fix must not
/// depend on the poll cadence.
#[test]
fn adjacent_repeats_are_cadence_independent() {
    let clips = vec![midi_clip(vec![
        note(60, 0, 40),
        note(60, 40, 40),
        note(60, 80, 40),
    ])];

    let coarse = run_windows(&clips, 8_000, 3_000);
    let fine = run_windows(&clips, 8_000, 137);

    assert_paired(&fine);
    assert_eq!(coarse, fine);
}

/// A short repeat *inside* a longer note of the same pitch: the long note
/// must be released when the short one retriggers, and the short one
/// released at its own end. Previously the long note's NoteOff was
/// overwritten and never sent.
#[test]
fn overlapping_same_pitch_releases_the_held_voice_first() {
    let clips = vec![midi_clip(vec![
        note(60, 0, 100),  // samples [0, 5_000)
        note(60, 20, 20),  // samples [1_000, 2_000)
    ])];

    let msgs = run_windows(&clips, 8_000, 700);

    assert_paired(&msgs);
    assert_eq!(
        msgs,
        vec![
            Msg::On(TRACK, 0, 60, 127),
            // The long note is released as the short one retriggers…
            Msg::Off(TRACK, 0, 60),
            Msg::On(TRACK, 0, 60, 127),
            // …and the long note's original end no longer fires a stale
            // NoteOff that would cut the short one short.
            Msg::Off(TRACK, 0, 60),
        ]
    );
}

/// Distinct pitches sharing a boundary are unaffected: no spurious
/// release, and the messages come out in timeline order.
#[test]
fn adjacent_distinct_pitches_are_untouched() {
    let clips = vec![midi_clip(vec![note(60, 0, 40), note(64, 40, 40)])];

    let msgs = run_windows(&clips, 8_000, 3_000);

    assert_paired(&msgs);
    assert_eq!(
        msgs,
        vec![
            Msg::On(TRACK, 0, 60, 127),
            Msg::Off(TRACK, 0, 60),
            Msg::On(TRACK, 0, 64, 127),
            Msg::Off(TRACK, 0, 64),
        ]
    );
}

/// A chord's notes all start and end together; each key still gets its
/// own paired on/off.
#[test]
fn chord_notes_each_get_their_own_pairing() {
    let clips = vec![midi_clip(vec![
        note(60, 0, 40),
        note(64, 0, 40),
        note(67, 0, 40),
    ])];

    let msgs = run_windows(&clips, 8_000, 3_000);

    assert_paired(&msgs);
    assert_eq!(msgs.len(), 6);
}

/// Messages inside one poll window come out ordered by timeline position,
/// not grouped as "every NoteOn, then every NoteOff" — a legato line
/// stays legato on the wire.
#[test]
fn one_window_emits_in_timeline_order() {
    let clips = vec![midi_clip(vec![
        note(60, 0, 10),  // [0, 500)
        note(62, 20, 10), // [1_000, 1_500)
        note(64, 40, 10), // [2_000, 2_500)
    ])];

    // One window covering the lot.
    let msgs = run_windows(&clips, 60 * SAMPLES_PER_TICK, 60 * SAMPLES_PER_TICK);

    assert_paired(&msgs);
    assert_eq!(
        msgs,
        vec![
            Msg::On(TRACK, 0, 60, 127),
            Msg::Off(TRACK, 0, 60),
            Msg::On(TRACK, 0, 62, 127),
            Msg::Off(TRACK, 0, 62),
            Msg::On(TRACK, 0, 64, 127),
            Msg::Off(TRACK, 0, 64),
        ]
    );
}
