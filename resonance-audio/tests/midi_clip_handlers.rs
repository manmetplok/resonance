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
use parking_lot::RwLock;

use resonance_audio::types::{AudioEvent, MidiClip, MidiNote};
use resonance_audio::{move_midi_clip_in_place, trim_midi_clip_in_place};

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
    let midi_clips: Arc<RwLock<Vec<MidiClip>>> =
        Arc::new(RwLock::new(vec![sample_clip(1, 100, 0)]));
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
    let clips = midi_clips.read();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].id, 1);
    assert_eq!(clips[0].start_sample, 0);
    assert_eq!(clips[0].track_id, 100);
}

#[test]
fn trim_missing_clip_emits_no_event() {
    let midi_clips: Arc<RwLock<Vec<MidiClip>>> =
        Arc::new(RwLock::new(vec![sample_clip(1, 100, 0)]));
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
    let clips = midi_clips.read();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].start_sample, 0);
    assert_eq!(clips[0].trim_start_ticks, 0);
    assert_eq!(clips[0].trim_end_ticks, 0);
}

#[test]
fn move_existing_clip_mutates_and_emits_event() {
    // Happy path companion to the missing-clip cases: prove the fix
    // didn't accidentally suppress the event for the real lookup hit.
    let midi_clips: Arc<RwLock<Vec<MidiClip>>> =
        Arc::new(RwLock::new(vec![sample_clip(7, 100, 0)]));
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

    let clips = midi_clips.read();
    assert_eq!(clips[0].start_sample, 96_000);
    assert_eq!(clips[0].track_id, 200);
}

#[test]
fn trim_existing_clip_mutates_and_emits_event() {
    let midi_clips: Arc<RwLock<Vec<MidiClip>>> =
        Arc::new(RwLock::new(vec![sample_clip(7, 100, 0)]));
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

    let clips = midi_clips.read();
    assert_eq!(clips[0].start_sample, 24_000);
    assert_eq!(clips[0].trim_start_ticks, 240);
    assert_eq!(clips[0].trim_end_ticks, 120);
}
