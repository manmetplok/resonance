//! Verifies that `collect_midi_events_bounce` returns every note in a
//! multi-note clip when called with successive chunk windows that span
//! the full clip — the path the offline bounce uses to feed events to
//! the source plugin one chunk at a time.

use resonance_audio::collect_midi_events_bounce;
use resonance_audio::types::{MidiClip, MidiNote, PendingNoteEvent, TempoMap, TICKS_PER_QUARTER_NOTE};

const SR: u32 = 48_000;
const CHUNK: usize = 1024;

fn flat_tempo_map(bpm: f32) -> TempoMap {
    let mut tm = TempoMap::default();
    tm.bpm = bpm;
    tm.numerator = 4;
    tm.denominator = 4;
    tm
}

/// Quarter-note clip with `count` consecutive notes starting at tick 0.
fn quarter_note_clip(count: usize, clip_start: u64) -> MidiClip {
    let q = TICKS_PER_QUARTER_NOTE;
    let notes: Vec<MidiNote> = (0..count)
        .map(|i| MidiNote {
            note: 60 + i as u8,
            velocity: 1.0,
            start_tick: i as u64 * q,
            duration_ticks: q,
        })
        .collect();
    MidiClip {
        id: 1,
        track_id: 42,
        start_sample: clip_start,
        duration_ticks: count as u64 * q,
        notes,
        name: "test".into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

fn collect_all_chunks(
    clips: &[MidiClip],
    tempo_map: &TempoMap,
    range_start: u64,
    range_end: u64,
) -> Vec<PendingNoteEvent> {
    let mut all = Vec::new();
    let mut buf: Vec<PendingNoteEvent> = Vec::new();
    let mut pos = range_start;
    while pos < range_end {
        let frames = ((range_end - pos) as usize).min(CHUNK);
        collect_midi_events_bounce(clips, /* track_id */ 42, pos, frames, tempo_map, SR, &mut buf);
        // Translate sample_offset back to absolute samples for assertion convenience.
        for ev in &buf {
            all.push(PendingNoteEvent {
                is_note_on: ev.is_note_on,
                note: ev.note,
                velocity: ev.velocity,
                sample_offset: ev.sample_offset + (pos - range_start) as u32,
            });
        }
        pos += frames as u64;
    }
    all
}

#[test]
fn four_note_clip_emits_all_note_ons_and_offs_across_chunks() {
    // 4 quarter notes at 120 BPM => 24_000 samples per beat. Total
    // visible audio is 4 beats = 96_000 samples; render through the
    // chunked collection path the bounce uses and verify every note
    // boundary appears exactly once.
    let tm = flat_tempo_map(120.0);
    let clip = quarter_note_clip(4, /* clip_start */ 0);
    let events = collect_all_chunks(&[clip], &tm, 0, 96_000 + 1);

    let note_ons: Vec<&PendingNoteEvent> = events.iter().filter(|e| e.is_note_on).collect();
    let note_offs: Vec<&PendingNoteEvent> = events.iter().filter(|e| !e.is_note_on).collect();

    assert_eq!(note_ons.len(), 4, "every NoteOn should fire once: {events:?}");
    assert_eq!(note_offs.len(), 4, "every NoteOff should fire once: {events:?}");

    // Each NoteOn should land at quarter-note boundaries.
    for (i, ev) in note_ons.iter().enumerate() {
        let expected = (i as u32) * 24_000;
        let diff = (ev.sample_offset as i64 - expected as i64).abs();
        assert!(diff <= 1, "NoteOn {i}: offset {} vs expected {expected}", ev.sample_offset);
        assert_eq!(ev.note, 60 + i as u8);
    }

    // The 4th NoteOff lands exactly at sample 96_000 — make sure the
    // half-open `[playhead, buf_end)` window doesn't drop boundary
    // events. (`collect_all_chunks` extends `range_end` by 1 to give
    // the boundary a chunk of its own.)
    for (i, ev) in note_offs.iter().enumerate() {
        let expected = ((i + 1) as u32) * 24_000;
        let diff = (ev.sample_offset as i64 - expected as i64).abs();
        assert!(diff <= 1, "NoteOff {i}: offset {} vs expected {expected}", ev.sample_offset);
    }
}

#[test]
fn note_off_sorts_before_note_on_at_same_sample_offset() {
    // Consecutive quarter notes share boundaries: note i's NoteOff and
    // note i+1's NoteOn land on the same sample. Collect the whole clip
    // in one call so all events get sorted together, and verify each
    // boundary keeps NoteOff before NoteOn — the pairing the engine
    // relies on for legato retriggers (previously guaranteed by the
    // stable sort, now by the (offset, is_note_on) key).
    let tm = flat_tempo_map(120.0);
    let clip = quarter_note_clip(4, /* clip_start */ 0);
    let mut events: Vec<PendingNoteEvent> = Vec::new();
    collect_midi_events_bounce(&[clip], 42, 0, 96_001, &tm, SR, &mut events);

    assert_eq!(events.len(), 8, "4 NoteOns + 4 NoteOffs: {events:?}");

    // Offsets must be non-decreasing.
    for pair in events.windows(2) {
        assert!(
            pair[0].sample_offset <= pair[1].sample_offset,
            "events not sorted by offset: {events:?}"
        );
    }

    // At each shared boundary the NoteOff must precede the NoteOn.
    for pair in events.windows(2) {
        if pair[0].sample_offset == pair[1].sample_offset {
            assert!(
                !pair[0].is_note_on && pair[1].is_note_on,
                "NoteOff must precede NoteOn at equal offsets: {events:?}"
            );
        }
    }
    let boundaries = events
        .windows(2)
        .filter(|p| p[0].sample_offset == p[1].sample_offset)
        .count();
    assert_eq!(boundaries, 3, "expected 3 shared on/off boundaries: {events:?}");
}

#[test]
fn clip_offset_from_zero_still_produces_every_note() {
    // Same 4-note clip, but parked at sample 100_000 — the offline
    // bounce starts rendering at the earliest MIDI clip start, so
    // chunk pos values land in the [100_000, 196_000) range. A bug
    // that compared playhead to absolute clip ticks would show only
    // the first note here.
    let tm = flat_tempo_map(120.0);
    let clip = quarter_note_clip(4, /* clip_start */ 100_000);
    let events = collect_all_chunks(&[clip], &tm, 100_000, 100_000 + 96_000 + 1);

    assert_eq!(
        events.iter().filter(|e| e.is_note_on).count(),
        4,
        "all 4 NoteOns missing for offset clip: {events:?}"
    );
    assert_eq!(events.iter().filter(|e| !e.is_note_on).count(), 4);
}
