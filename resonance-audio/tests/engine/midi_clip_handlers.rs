//! Regression for the `MidiClipMoved` / `MidiClipTrimmed` ghost-event
//! bug: the handlers used to emit the event unconditionally, so issuing
//! a `MoveMidiClip` / `TrimMidiClip` for an unknown clip id would still
//! tell the app the move/trim happened — corrupting the UI mirror,
//! poisoning the undo stack, and dirtying the project. Fixed by folding
//! mutation and event emission into a single `if let Some(clip)` branch
//! (matching the audio-clip handlers).
//!
//! Drives the engine-internal pure helpers
//! [`move_midi_clip_in_place`] / [`trim_midi_clip_in_place`] directly via
//! `#[doc(hidden)]` re-exports. That keeps the test headless — no cpal
//! stream, no engine thread, no audio device — while still exercising the
//! exact code that the `AudioCommand::MoveMidiClip` /
//! `AudioCommand::TrimMidiClip` dispatch path runs.

use std::sync::Arc;

use crossbeam_channel::unbounded;

use resonance_audio::test_support::SharedState;
use resonance_audio::types::{AudioEvent, MidiClip, MidiNote};
use resonance_audio::{move_midi_clip_in_place, trim_midi_clip_in_place};

/// Engine state holding `clip` in its published render graph.
fn shared_with(clip: MidiClip) -> SharedState {
    let shared = SharedState::default();
    shared.edit_midi_clips(|clips| clips.push(Arc::new(clip)));
    shared
}

fn sample_clip(id: u64, track_id: u64, start_sample: u64) -> MidiClip {
    MidiClip {
        id,
        track_id,
        start_sample,
        duration_ticks: 1920,
        notes: vec![MidiNote {
            note: 60,
            velocity: 0.8,
            start_tick: 0,
            duration_ticks: 480,
        }],
        name: "clip".into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

#[test]
fn move_missing_clip_emits_no_event() {
    let midi_clips = shared_with(sample_clip(1, 100, 0));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    // Clip id 999 does not exist — the handler must be a no-op and emit
    // nothing.
    move_midi_clip_in_place(
        &midi_clips,
        &event_tx,
        /* clip_id */ 999,
        /* new_start_sample */ 48_000,
        /* new_track_id */ 200,
    );

    assert!(
        event_rx.try_recv().is_err(),
        "MidiClipMoved must not be emitted when the clip lookup misses"
    );
    // The existing clip must be untouched.
    let clips = midi_clips.graph.load().midi_clips.clone();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].id, 1);
    assert_eq!(clips[0].start_sample, 0);
    assert_eq!(clips[0].track_id, 100);
}

#[test]
fn trim_missing_clip_emits_no_event() {
    let midi_clips = shared_with(sample_clip(1, 100, 0));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    trim_midi_clip_in_place(
        &midi_clips,
        &event_tx,
        /* clip_id */ 999,
        /* new_start_sample */ 48_000,
        /* trim_start_ticks */ 240,
        /* trim_end_ticks */ 120,
    );

    assert!(
        event_rx.try_recv().is_err(),
        "MidiClipTrimmed must not be emitted when the clip lookup misses"
    );
    let clips = midi_clips.graph.load().midi_clips.clone();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].start_sample, 0);
    assert_eq!(clips[0].trim_start_ticks, 0);
    assert_eq!(clips[0].trim_end_ticks, 0);
}

#[test]
fn move_existing_clip_mutates_and_emits_event() {
    // Happy path companion to the missing-clip cases: prove the fix
    // didn't accidentally suppress the event for the real lookup hit.
    let midi_clips = shared_with(sample_clip(7, 100, 0));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    move_midi_clip_in_place(
        &midi_clips,
        &event_tx,
        /* clip_id */ 7,
        /* new_start_sample */ 96_000,
        /* new_track_id */ 200,
    );

    match event_rx.try_recv() {
        Ok(AudioEvent::MidiClipMoved {
            clip_id,
            new_start_sample,
            new_track_id,
        }) => {
            assert_eq!(clip_id, 7);
            assert_eq!(new_start_sample, 96_000);
            assert_eq!(new_track_id, 200);
        }
        other => panic!("expected MidiClipMoved, got {other:?}"),
    }
    assert!(
        event_rx.try_recv().is_err(),
        "exactly one event should be emitted"
    );

    let clips = midi_clips.graph.load().midi_clips.clone();
    assert_eq!(clips[0].start_sample, 96_000);
    assert_eq!(clips[0].track_id, 200);
}

#[test]
fn trim_existing_clip_mutates_and_emits_event() {
    let midi_clips = shared_with(sample_clip(7, 100, 0));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    trim_midi_clip_in_place(
        &midi_clips,
        &event_tx,
        /* clip_id */ 7,
        /* new_start_sample */ 24_000,
        /* trim_start_ticks */ 240,
        /* trim_end_ticks */ 120,
    );

    match event_rx.try_recv() {
        Ok(AudioEvent::MidiClipTrimmed {
            clip_id,
            new_start_sample,
            trim_start_ticks,
            trim_end_ticks,
        }) => {
            assert_eq!(clip_id, 7);
            assert_eq!(new_start_sample, 24_000);
            assert_eq!(trim_start_ticks, 240);
            assert_eq!(trim_end_ticks, 120);
        }
        other => panic!("expected MidiClipTrimmed, got {other:?}"),
    }
    assert!(
        event_rx.try_recv().is_err(),
        "exactly one event should be emitted"
    );

    let clips = midi_clips.graph.load().midi_clips.clone();
    assert_eq!(clips[0].start_sample, 24_000);
    assert_eq!(clips[0].trim_start_ticks, 240);
    assert_eq!(clips[0].trim_end_ticks, 120);
}

// -- Note moves re-sort the clip (code review VIEW-02 / CTL-02) ----------

fn n(pitch: u8, start: u64) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: 0.8,
        start_tick: start,
        duration_ticks: 240,
    }
}

/// `move_note_resorted` returns where a stable re-sort puts the moved note,
/// including among notes that start on the same tick.
#[test]
fn move_note_resorted_reports_the_moved_notes_new_index() {
    use resonance_audio::types::move_note_resorted;
    let base = vec![n(60, 0), n(62, 480), n(64, 480), n(65, 960)];
    // (from, new_start, expected index)
    for (from, to, want) in [
        (0, 600, 2),  // past both 480s
        (0, 480, 0),  // equal start: stays ahead of the notes it preceded
        (3, 480, 3),  // equal start: stays behind the notes it followed
        (3, 0, 1),    // equal start with note 0, which preceded it
        (1, 2000, 3), // to the end
        (2, 0, 1),
    ] {
        let mut notes = base.clone();
        let moved = notes[from].note;
        let got = move_note_resorted(&mut notes, from, to, moved);
        assert_eq!(got, want, "move {from} -> tick {to}");
        assert_eq!((notes[got].note, notes[got].start_tick), (moved, to));
        assert!(notes.windows(2).all(|w| w[0].start_tick <= w[1].start_tick));
    }
}

/// A drag that crosses a neighbour, replayed through the real engine
/// handler the way the editors now send it: each step addresses the note
/// by the index the previous step reported. The neighbour never moves.
#[test]
fn chained_note_moves_follow_the_dragged_note_across_a_neighbour() {
    use resonance_audio::test_support::EngineHandlerHarness;
    use resonance_audio::types::{move_note_resorted, AudioCommand};
    let mut h = EngineHandlerHarness::new();
    let mut clip = sample_clip(1, 1, 0);
    clip.notes = vec![n(60, 0), n(62, 480)];
    h.push_midi_clip(clip);

    let mut mirror = h.midi_notes(1);
    let mut index = 0;
    for tick in [300, 600, 700] {
        assert!(h.replay_midi_note_command(&AudioCommand::MoveMidiNote {
            clip_id: 1,
            note_index: index,
            new_start_tick: tick,
            new_note: 60,
        }));
        index = move_note_resorted(&mut mirror, index, tick, 60);
    }
    let got: Vec<(u8, u64)> = h.midi_notes(1).iter().map(|x| (x.note, x.start_tick)).collect();
    assert_eq!(got, vec![(62, 480), (60, 700)], "B stays put, A lands at 700");
    assert_eq!(index, 1);
}
