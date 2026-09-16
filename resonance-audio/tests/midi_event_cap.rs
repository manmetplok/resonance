//! Regression tests for the per-buffer MIDI event cap
//! (`mixer::midi_events`): when a block produces more events than
//! `MAX_MIDI_EVENTS_PER_BUFFER`, note-offs must survive the truncation.
//! The old behaviour stopped collecting at the cap outright, so a voice
//! whose note-on had already reached the plugin in an earlier block could
//! lose its release and stick until the next transport stop.

use resonance_audio::collect_midi_events_bounce;
use resonance_audio::types::*;

const SR: u32 = 48_000;

/// Mirror of `limits::MAX_MIDI_EVENTS_PER_BUFFER` (not exported). At
/// 120 BPM the numbers below assume a 50 samples-per-tick grid; if the
/// cap changes, the counts here need revisiting too.
const CAP: usize = 512;

fn flat_map() -> TempoMap {
    let mut tm = TempoMap::default();
    tm.bpm = 120.0;
    tm.numerator = 4;
    tm.denominator = 4;
    tm.rebuild_bar_table(SR);
    tm
}

fn clip_with(notes: Vec<MidiNote>) -> MidiClip {
    MidiClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        duration_ticks: 16 * 4 * TICKS_PER_QUARTER_NOTE,
        notes,
        name: "cap".to_string(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

#[test]
fn overflow_keeps_every_note_off() {
    // 120 BPM at 48 kHz is 50 samples per tick, so a 4800-frame block
    // spans 96 ticks. Block 2 covers samples [4800, 9600) = ticks
    // [96, 192).
    let frames = 4800usize;

    // 300 short notes entirely inside block 2: 600 events, well past the
    // 512-event cap. Iterated first, so they fill the buffer before the
    // long note below is reached.
    let mut notes: Vec<MidiNote> = (0..300)
        .map(|i| MidiNote {
            note: 30 + (i % 60) as u8,
            velocity: 0.8,
            start_tick: 96 + (i % 90) as u64,
            duration_ticks: 1,
        })
        .collect();
    // One long note whose note-on went out in block 1 (tick 10, sample
    // 500) and whose release lands in block 2 (tick 160, sample 8000).
    // Under the old truncation its note-off was dropped — a stuck note.
    notes.push(MidiNote {
        note: 100,
        velocity: 0.9,
        start_tick: 10,
        duration_ticks: 150,
    });

    let clips = [clip_with(notes)];
    let mut out: Vec<PendingNoteEvent> = Vec::new();
    collect_midi_events_bounce(&clips, 1, 4800, frames, &flat_map(), SR, &mut out);

    // The buffer never exceeds the cap…
    assert_eq!(out.len(), CAP);
    // …and every one of the 301 note-offs due in this block survived;
    // only note-ons were shed (211 of them evicted or dropped).
    let offs = out.iter().filter(|e| !e.is_note_on).count();
    assert_eq!(offs, 301, "note-offs were truncated at the event cap");
    assert!(
        out.iter().any(|e| !e.is_note_on && e.note == 100),
        "the long note's release was truncated — stuck note"
    );
    // Output stays sorted by sample offset (CLAP ordering contract).
    assert!(out
        .windows(2)
        .all(|w| w[0].sample_offset <= w[1].sample_offset));
}

#[test]
fn under_cap_collection_is_unchanged() {
    // Sanity guard on the capped push: a block comfortably under the cap
    // emits every on/off pair exactly as before.
    let notes: Vec<MidiNote> = (0..10)
        .map(|i| MidiNote {
            note: 60 + i as u8,
            velocity: 0.8,
            start_tick: 96 + i as u64,
            duration_ticks: 2,
        })
        .collect();

    let clips = [clip_with(notes)];
    let mut out: Vec<PendingNoteEvent> = Vec::new();
    collect_midi_events_bounce(&clips, 1, 4800, 4800, &flat_map(), SR, &mut out);

    assert_eq!(out.len(), 20);
    assert_eq!(out.iter().filter(|e| e.is_note_on).count(), 10);
    assert_eq!(out.iter().filter(|e| !e.is_note_on).count(), 10);
}
