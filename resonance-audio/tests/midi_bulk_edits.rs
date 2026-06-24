//! Engine-handler tests for the bulk MIDI edits (quantize / humanize /
//! groove). These drive the engine-internal `*_in_place` helpers directly
//! via `#[doc(hidden)]` re-exports — the exact code the
//! `AudioCommand::QuantizeMidiNotes` / `HumanizeMidiNotes` /
//! `ApplyGrooveToClip` / `ExtractGrooveFromClip` dispatch arms run — so
//! the tests stay headless: no cpal stream, no engine thread, no audio
//! device.
//!
//! The contract verified here:
//! * each op is **atomic** — the clip's note array is replaced in one
//!   locked pass and a single bulk event is emitted;
//! * the **selection** (note indices) is respected; unselected notes are
//!   untouched;
//! * `MidiNotesEdited` carries the **full resulting note array** so the
//!   app can mirror it and record the prior notes for undo;
//! * extraction emits `GrooveExtracted` and never mutates the clip;
//! * a **missing clip** is a no-op that emits no ghost event.

use std::sync::Arc;

use crossbeam_channel::unbounded;
use parking_lot::RwLock;

use resonance_audio::quantize::{Division, GridValue, GrooveTemplate, QuantizeMode};
use resonance_audio::types::{AudioEvent, MidiClip, MidiNote, TempoMap};
use resonance_audio::{
    apply_groove_to_clip_in_place, extract_groove_from_clip_in_place, humanize_midi_notes_in_place,
    quantize_midi_notes_in_place,
};

const SR: u32 = 48_000;

fn note(start: u64, dur: u64, vel: f32, pitch: u8) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: vel,
        start_tick: start,
        duration_ticks: dur,
    }
}

fn clip_with(id: u64, notes: Vec<MidiNote>) -> MidiClip {
    MidiClip {
        id,
        track_id: 100,
        start_sample: 0,
        duration_ticks: 1920,
        notes,
        name: "clip".into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

fn clips_of(clip: MidiClip) -> Arc<RwLock<Vec<MidiClip>>> {
    Arc::new(RwLock::new(vec![clip]))
}

// ---------------------------------------------------------------------
// Quantize
// ---------------------------------------------------------------------

#[test]
fn quantize_snaps_selected_and_emits_one_bulk_event() {
    // Notes slightly off a 16th grid (120 ticks): 5→0, 118→120, 250→240.
    let clips = clips_of(clip_with(
        1,
        vec![
            note(5, 100, 0.8, 60),
            note(118, 100, 0.8, 62),
            note(250, 100, 0.8, 64),
        ],
    ));
    let (tx, rx) = unbounded::<AudioEvent>();
    let tempo = TempoMap::default();

    quantize_midi_notes_in_place(
        &clips,
        &tx,
        &tempo,
        SR,
        /* clip_id */ 1,
        /* indices */ &[0, 1, 2],
        Division::straight(GridValue::Sixteenth),
        /* strength */ 1.0,
        /* swing */ 0.0,
        QuantizeMode::StartOnly,
        /* quantize_ends */ false,
        /* iterative */ false,
    );

    // Engine clip table mutated to the snapped positions.
    {
        let guard = clips.read();
        let n = &guard[0].notes;
        assert_eq!(n[0].start_tick, 0);
        assert_eq!(n[1].start_tick, 120);
        assert_eq!(n[2].start_tick, 240);
        // StartOnly leaves durations alone.
        assert_eq!(n[0].duration_ticks, 100);
    }

    // Exactly one bulk event, carrying the full resulting note array.
    match rx.try_recv() {
        Ok(AudioEvent::MidiNotesEdited { clip_id, notes }) => {
            assert_eq!(clip_id, 1);
            assert_eq!(notes.len(), 3);
            assert_eq!(notes[0].start_tick, 0);
            assert_eq!(notes[1].start_tick, 120);
            assert_eq!(notes[2].start_tick, 240);
        }
        other => panic!("expected one MidiNotesEdited, got {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "exactly one bulk event per op");
}

#[test]
fn quantize_respects_selection_indices() {
    let clips = clips_of(clip_with(
        1,
        vec![
            note(5, 100, 0.8, 60),   // selected -> 0
            note(118, 100, 0.8, 62), // NOT selected -> unchanged (118)
            note(250, 100, 0.8, 64), // selected -> 240
        ],
    ));
    let (tx, _rx) = unbounded::<AudioEvent>();
    let tempo = TempoMap::default();

    quantize_midi_notes_in_place(
        &clips,
        &tx,
        &tempo,
        SR,
        1,
        &[0, 2],
        Division::straight(GridValue::Sixteenth),
        1.0,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
    );

    let guard = clips.read();
    let n = &guard[0].notes;
    assert_eq!(n[0].start_tick, 0, "selected note snapped");
    assert_eq!(n[1].start_tick, 118, "unselected note untouched");
    assert_eq!(n[2].start_tick, 240, "selected note snapped");
}

#[test]
fn quantize_missing_clip_is_noop_no_event() {
    let clips = clips_of(clip_with(1, vec![note(5, 100, 0.8, 60)]));
    let (tx, rx) = unbounded::<AudioEvent>();
    let tempo = TempoMap::default();

    quantize_midi_notes_in_place(
        &clips,
        &tx,
        &tempo,
        SR,
        /* clip_id */ 999,
        &[0],
        Division::straight(GridValue::Sixteenth),
        1.0,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
    );

    assert!(
        rx.try_recv().is_err(),
        "missing clip must emit no ghost event"
    );
    assert_eq!(
        clips.read()[0].notes[0].start_tick,
        5,
        "missing-clip op must not mutate any clip"
    );
}

// ---------------------------------------------------------------------
// Humanize
// ---------------------------------------------------------------------

#[test]
fn humanize_is_deterministic_and_emits_bulk_event() {
    let make = || clips_of(clip_with(1, vec![note(480, 200, 0.8, 60), note(960, 200, 0.5, 64)]));
    let a = make();
    let b = make();
    let (txa, rxa) = unbounded::<AudioEvent>();
    let (txb, _rxb) = unbounded::<AudioEvent>();

    humanize_midi_notes_in_place(&a, &txa, 1, &[0, 1], 20, 0.2, /* seed */ 42);
    humanize_midi_notes_in_place(&b, &txb, 1, &[0, 1], 20, 0.2, /* seed */ 42);

    // Same seed → identical result (reproducible / undoable).
    let na = a.read();
    let nb = b.read();
    for (x, y) in na[0].notes.iter().zip(nb[0].notes.iter()) {
        assert_eq!(x.start_tick, y.start_tick);
        assert_eq!(x.velocity, y.velocity);
    }

    match rxa.try_recv() {
        Ok(AudioEvent::MidiNotesEdited { clip_id, notes }) => {
            assert_eq!(clip_id, 1);
            assert_eq!(notes.len(), 2);
        }
        other => panic!("expected MidiNotesEdited, got {other:?}"),
    }
    assert!(rxa.try_recv().is_err());
}

#[test]
fn humanize_missing_clip_is_noop_no_event() {
    let clips = clips_of(clip_with(1, vec![note(480, 200, 0.8, 60)]));
    let (tx, rx) = unbounded::<AudioEvent>();

    humanize_midi_notes_in_place(&clips, &tx, /* clip_id */ 7, &[0], 20, 0.2, 1);

    assert!(rx.try_recv().is_err());
    assert_eq!(clips.read()[0].notes[0].start_tick, 480);
}

// ---------------------------------------------------------------------
// Groove apply / extract
// ---------------------------------------------------------------------

/// A 16-step 4/4 groove that delays every odd 16th by 30 ticks.
fn swing_template() -> GrooveTemplate {
    let mut t = GrooveTemplate::identity(16);
    for step in (1..16).step_by(2) {
        t.timing_offsets_ticks[step] = 30;
    }
    t
}

#[test]
fn apply_groove_shifts_offbeats_and_emits_bulk_event() {
    // Note on the off-16th (step 1 = 120 ticks) gets delayed by 30.
    let clips = clips_of(clip_with(
        1,
        vec![note(0, 100, 0.8, 60), note(120, 100, 0.8, 62)],
    ));
    let (tx, rx) = unbounded::<AudioEvent>();
    let tempo = TempoMap::default();

    apply_groove_to_clip_in_place(&clips, &tx, &tempo, 1, &[0, 1], &swing_template(), 1.0);

    {
        let guard = clips.read();
        let n = &guard[0].notes;
        assert_eq!(n[0].start_tick, 0, "downbeat unchanged");
        assert_eq!(n[1].start_tick, 150, "off-16th delayed by 30");
    }
    match rx.try_recv() {
        Ok(AudioEvent::MidiNotesEdited { clip_id, notes }) => {
            assert_eq!(clip_id, 1);
            assert_eq!(notes.len(), 2);
            assert_eq!(notes[1].start_tick, 150);
        }
        other => panic!("expected MidiNotesEdited, got {other:?}"),
    }
    assert!(rx.try_recv().is_err());
}

#[test]
fn apply_groove_missing_clip_is_noop_no_event() {
    let clips = clips_of(clip_with(1, vec![note(120, 100, 0.8, 62)]));
    let (tx, rx) = unbounded::<AudioEvent>();
    let tempo = TempoMap::default();

    apply_groove_to_clip_in_place(&clips, &tx, &tempo, /* clip_id */ 5, &[0], &swing_template(), 1.0);

    assert!(rx.try_recv().is_err());
    assert_eq!(clips.read()[0].notes[0].start_tick, 120);
}

#[test]
fn extract_groove_emits_template_without_mutating_clip() {
    let clips = clips_of(clip_with(
        1,
        vec![note(0, 100, 0.8, 60), note(150, 100, 0.8, 62)],
    ));
    let (tx, rx) = unbounded::<AudioEvent>();
    let tempo = TempoMap::default();

    extract_groove_from_clip_in_place(
        &clips,
        &tx,
        &tempo,
        1,
        Division::straight(GridValue::Sixteenth),
    );

    // The clip is read-only for extraction.
    {
        let guard = clips.read();
        assert_eq!(guard[0].notes[0].start_tick, 0);
        assert_eq!(guard[0].notes[1].start_tick, 150);
    }
    match rx.try_recv() {
        Ok(AudioEvent::GrooveExtracted { template }) => {
            assert_eq!(template.steps_per_bar, 16);
            assert_eq!(template.timing_offsets_ticks.len(), 16);
            assert_eq!(template.velocity_scale.len(), 16);
        }
        other => panic!("expected GrooveExtracted, got {other:?}"),
    }
    assert!(rx.try_recv().is_err());
}

#[test]
fn extract_groove_missing_clip_is_noop_no_event() {
    let clips = clips_of(clip_with(1, vec![note(0, 100, 0.8, 60)]));
    let (tx, rx) = unbounded::<AudioEvent>();
    let tempo = TempoMap::default();

    extract_groove_from_clip_in_place(
        &clips,
        &tx,
        &tempo,
        /* clip_id */ 404,
        Division::straight(GridValue::Sixteenth),
    );

    assert!(rx.try_recv().is_err());
}
