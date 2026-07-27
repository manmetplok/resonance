//! Bulk-MIDI-edit engine-event mirroring (ba todo #390).
//!
//! Drives `MidiNotesEdited` / `GrooveExtracted` `AudioEvent`s through the
//! real dispatch and asserts the app mirrors them into `ClipState` /
//! the groove library wholesale — no per-note event churn, no read setters.

use resonance_app::Resonance;
use resonance_audio::quantize::GrooveTemplate;
use resonance_audio::types::{AudioEvent, MidiNote};

fn note(note: u8, start_tick: u64, velocity: f32) -> MidiNote {
    MidiNote {
        note,
        velocity,
        start_tick,
        duration_ticks: 120,
    }
}

/// Seed a MIDI clip the way the live app would — via the engine event,
/// not a private setter.
fn create_clip(app: &mut Resonance, clip_id: u64, notes: Vec<MidiNote>) {
    app.test_apply_engine_event(AudioEvent::MidiClipCreated {
        clip_id,
        track_id: 1,
        start_sample: 0,
        duration_ticks: 1920,
        name: format!("Clip {clip_id}"),
        notes,
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
}

#[test]
fn notes_edited_replaces_the_clip_note_vector_wholesale() {
    let (mut app, _task) = Resonance::new();
    create_clip(&mut app, 7, vec![note(60, 5, 0.5), note(64, 245, 0.9)]);

    // Quantize result: same two notes snapped to the grid, velocities humanized.
    let quantized = vec![note(60, 0, 0.6), note(64, 240, 0.8)];
    app.test_apply_engine_event(AudioEvent::MidiNotesEdited {
        clip_id: 7,
        notes: quantized.clone(),
    });

    let clip = app
        .test_midi_clips()
        .iter()
        .find(|c| c.id == 7)
        .expect("clip 7 exists");
    assert_eq!(clip.notes.len(), 2);
    assert_eq!(clip.notes[0].start_tick, 0);
    assert_eq!(clip.notes[1].start_tick, 240);
    assert_eq!(clip.notes[0].velocity, 0.6);
    assert_eq!(clip.notes[1].velocity, 0.8);
}

#[test]
fn notes_edited_for_unknown_clip_is_a_noop() {
    let (mut app, _task) = Resonance::new();
    create_clip(&mut app, 1, vec![note(60, 0, 0.7)]);

    // No clip 99 — must not panic and must not invent a clip.
    app.test_apply_engine_event(AudioEvent::MidiNotesEdited {
        clip_id: 99,
        notes: vec![note(72, 0, 1.0)],
    });

    assert_eq!(app.test_midi_clips().len(), 1);
    assert!(app.test_midi_clips().iter().all(|c| c.id != 99));
    // The existing clip is untouched.
    let clip = &app.test_midi_clips()[0];
    assert_eq!(clip.notes.len(), 1);
    assert_eq!(clip.notes[0].note, 60);
}

#[test]
fn notes_edited_can_grow_and_shrink_the_note_count() {
    let (mut app, _task) = Resonance::new();
    create_clip(&mut app, 3, vec![note(60, 0, 0.7)]);

    // Grow to three notes.
    app.test_apply_engine_event(AudioEvent::MidiNotesEdited {
        clip_id: 3,
        notes: vec![note(60, 0, 0.7), note(62, 120, 0.7), note(64, 240, 0.7)],
    });
    assert_eq!(app.test_midi_clips()[0].notes.len(), 3);

    // Shrink back to one.
    app.test_apply_engine_event(AudioEvent::MidiNotesEdited {
        clip_id: 3,
        notes: vec![note(67, 0, 0.7)],
    });
    let clip = &app.test_midi_clips()[0];
    assert_eq!(clip.notes.len(), 1);
    assert_eq!(clip.notes[0].note, 67);
}

#[test]
fn groove_extracted_appends_to_the_groove_library() {
    let (mut app, _task) = Resonance::new();
    assert!(app.test_groove_library().is_empty());

    let template = GrooveTemplate {
        steps_per_bar: 4,
        timing_offsets_ticks: vec![0, 12, -6, 4],
        velocity_scale: vec![1.0, 0.9, 1.1, 0.95],
    };
    app.test_apply_engine_event(AudioEvent::GrooveExtracted {
        template: template.clone(),
    });

    let lib = app.test_groove_library();
    assert_eq!(lib.len(), 1);
    assert_eq!(lib[0].steps_per_bar, 4);
    assert_eq!(lib[0].timing_offsets_ticks, vec![0, 12, -6, 4]);

    // A second extraction grows the library; templates accumulate.
    app.test_apply_engine_event(AudioEvent::GrooveExtracted {
        template: GrooveTemplate::identity(8),
    });
    assert_eq!(app.test_groove_library().len(), 2);
    assert_eq!(app.test_groove_library()[1].steps_per_bar, 8);
}
