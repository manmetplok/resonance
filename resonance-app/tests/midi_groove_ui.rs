//! Groove extract / apply panel wiring (ba todo #394, doc #163, epic #25).
//!
//! Covers the view-driven groove slice that sits beneath the Quantize and
//! Humanize rows in the MIDI editor:
//!
//! * the pure panel setters (`SetGrooveName` / `SetGrooveSelection` /
//!   `SetGrooveStrength`) that the picker, name field and strength slider
//!   write,
//! * naming an extracted groove into the **project** groove library when
//!   the `GrooveExtracted` engine event lands (#390 mirror), and
//! * applying a user-extracted groove by id — the round trip that lets a
//!   feel captured from one clip be dropped onto another.
//!
//! The engine is swapped for a command-capturing stub so the apply path
//! asserts the exact `AudioCommand::ApplyGrooveToClip` carrying the
//! resolved template.

use resonance_app::message::{Message, MidiEditorMessage};
use resonance_app::state::{GrooveSelection, MidiClipState};
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::quantize::{Division, GridValue, GrooveTemplate};
use resonance_audio::types::{AudioCommand, AudioEvent, ClipId, MidiNote};

const CLIP: ClipId = 7;

fn note(start_tick: u64) -> MidiNote {
    MidiNote {
        note: 60,
        velocity: 0.8,
        start_tick,
        duration_ticks: 120,
    }
}

fn app_with_open_clip(count: usize) -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new();
    let rx = app.test_capture_engine();
    app.test_push_midi_clip(MidiClipState {
        id: CLIP,
        track_id: 1,
        start_sample: 0,
        duration_ticks: 4 * 480,
        name: "test".to_string(),
        notes: (0..count).map(|i| note(i as u64 * 117)).collect(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::OpenMidiEditor(CLIP)));
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn grid() -> Division {
    Division::straight(GridValue::Sixteenth)
}

/// A recognisable, non-identity template to stand in for an extraction.
fn sample_template() -> GrooveTemplate {
    let mut t = GrooveTemplate::identity(16);
    t.timing_offsets_ticks[1] = 37;
    t.velocity_scale[1] = 0.8;
    t
}

// ---------------------------------------------------------------------
// Panel setters
// ---------------------------------------------------------------------

#[test]
fn groove_setters_update_each_bound_field() {
    let (mut app, _rx) = app_with_open_clip(1);

    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::SetGrooveName(
        "Funky".to_string(),
    )));
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::SetGrooveSelection(
        GrooveSelection::Stock { index: 2 },
    )));
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::SetGrooveStrength(
        0.5,
    )));

    let panel = app.test_quantize_panel();
    assert_eq!(panel.groove_name, "Funky");
    assert_eq!(panel.groove_selection, GrooveSelection::Stock { index: 2 });
    assert!((panel.groove_strength - 0.5).abs() < 1e-6);
}

#[test]
fn groove_strength_is_clamped_to_unit_range() {
    let (mut app, _rx) = app_with_open_clip(1);
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::SetGrooveStrength(
        4.2,
    )));
    assert!((app.test_quantize_panel().groove_strength - 1.0).abs() < 1e-6);
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::SetGrooveStrength(
        -1.0,
    )));
    assert!(app.test_quantize_panel().groove_strength.abs() < 1e-6);
}

// ---------------------------------------------------------------------
// Extract → name into the project groove library
// ---------------------------------------------------------------------

#[test]
fn extract_files_named_groove_into_project_library() {
    let (mut app, rx) = app_with_open_clip(4);
    let _ = drain(&rx);

    // Name it, extract (sends the command + stashes the pending name)…
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::SetGrooveName(
        "My Feel".to_string(),
    )));
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::ExtractGroove {
        grid: grid(),
    }));
    // …then the engine answers with the captured template.
    let template = sample_template();
    app.test_apply_engine_event(AudioEvent::GrooveExtracted {
        template: template.clone(),
    });

    let lib = &app.test_quantize().groove_library;
    assert_eq!(lib.len(), 1, "one user groove filed");
    assert_eq!(lib[0].id, 0);
    assert_eq!(lib[0].name, "My Feel");
    assert_eq!(lib[0].template, template);
    // The pending name is consumed so a second extract can't reuse it.
    assert!(app.test_quantize_panel().pending_groove_name.is_none());
}

#[test]
fn extract_without_a_name_gets_an_auto_default() {
    let (mut app, rx) = app_with_open_clip(4);
    let _ = drain(&rx);

    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::ExtractGroove {
        grid: grid(),
    }));
    app.test_apply_engine_event(AudioEvent::GrooveExtracted {
        template: sample_template(),
    });

    let lib = &app.test_quantize().groove_library;
    assert_eq!(lib.len(), 1);
    assert_eq!(lib[0].name, "Groove 1", "blank name ⇒ auto-numbered");
}

#[test]
fn extracted_groove_ids_stay_unique() {
    let (mut app, rx) = app_with_open_clip(4);
    let _ = drain(&rx);

    for _ in 0..3 {
        app.test_dispatch(Message::MidiEditor(MidiEditorMessage::ExtractGroove {
            grid: grid(),
        }));
        app.test_apply_engine_event(AudioEvent::GrooveExtracted {
            template: sample_template(),
        });
    }

    let ids: Vec<u64> = app.test_quantize().groove_library.iter().map(|g| g.id).collect();
    assert_eq!(ids, vec![0, 1, 2]);
}

// ---------------------------------------------------------------------
// Apply a user-extracted groove by id
// ---------------------------------------------------------------------

#[test]
fn apply_user_groove_emits_command_with_its_template() {
    let (mut app, rx) = app_with_open_clip(4);
    let _ = drain(&rx);

    // Seed a user groove (id 0) directly into the project library.
    let template = sample_template();
    app.test_quantize_mut()
        .groove_library
        .push(resonance_app::state::UserGroove {
            id: 0,
            name: "Captured".to_string(),
            template: template.clone(),
        });

    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::ApplyGroove {
        template_id: "user:0".to_string(),
        strength: 0.6,
    }));

    match drain(&rx).as_slice() {
        [AudioCommand::ApplyGrooveToClip {
            clip_id,
            template: applied,
            strength,
            ..
        }] if *clip_id == CLIP => {
            assert_eq!(*applied, template);
            assert!((*strength - 0.6).abs() < 1e-6);
        }
        other => panic!("expected one ApplyGrooveToClip, got {other:?}"),
    }
}

#[test]
fn apply_unknown_user_groove_is_a_noop() {
    let (mut app, rx) = app_with_open_clip(4);
    let _ = drain(&rx);
    // No groove with id 9 exists ⇒ no command, no undo entry.
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::ApplyGroove {
        template_id: "user:9".to_string(),
        strength: 1.0,
    }));
    assert!(drain(&rx).is_empty());
}

/// The headline DoD: a groove extracted from one clip can be applied to
/// another. Extract (clip open) → event fills the library → apply the
/// `user:<id>` template carries the captured feel back to the engine.
#[test]
fn extracted_groove_can_be_applied() {
    let (mut app, rx) = app_with_open_clip(4);
    let _ = drain(&rx);

    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::SetGrooveName(
        "Roundtrip".to_string(),
    )));
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::ExtractGroove {
        grid: grid(),
    }));
    let _ = drain(&rx); // discard the ExtractGrooveFromClip command
    let template = sample_template();
    app.test_apply_engine_event(AudioEvent::GrooveExtracted {
        template: template.clone(),
    });

    let id = app.test_quantize().groove_library[0].id;
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::ApplyGroove {
        template_id: format!("user:{id}"),
        strength: 1.0,
    }));

    match drain(&rx).as_slice() {
        [AudioCommand::ApplyGrooveToClip { template: applied, .. }] => {
            assert_eq!(*applied, template, "applied the extracted feel");
        }
        other => panic!("expected one ApplyGrooveToClip, got {other:?}"),
    }
}
