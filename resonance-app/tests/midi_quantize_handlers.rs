//! Bulk MIDI timing-edit update handlers + undo wiring (ba todo #391,
//! doc #163, epic #25).
//!
//! Each bulk op — Quantize / Humanize / ApplyGroove / ExtractGroove —
//! is dispatched against a `Resonance` whose engine has been swapped for
//! a command-capturing stub, so the tests assert the exact `AudioCommand`
//! the handler emits. These messages carry no `clip_id`: the handler
//! reads the *open* editor clip and the current multi-note selection
//! (#389), falling back to the whole clip when nothing is selected. The
//! engine applies the op and mirrors a bulk `MidiNotesEdited` back (#388
//! / #390); here we only verify the command + index gathering, plus the
//! undo classification that makes each op a single undo step.

use resonance_app::message::{Message, MidiEditorMessage};
use resonance_app::state::MidiClipState;
use resonance_app::undo::{classify, UndoAction};
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::quantize::{stock_grooves, Division, GridValue, QuantizeMode};
use resonance_audio::types::{AudioCommand, ClipId, MidiNote};

const CLIP: ClipId = 7;

fn note(start_tick: u64) -> MidiNote {
    MidiNote {
        note: 60,
        velocity: 0.8,
        start_tick,
        duration_ticks: 120,
    }
}

/// App with a capturing engine and one open MIDI clip holding `count`
/// notes (note `i` starts a little off the grid). Returns the app + the
/// command receiver.
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
    // Open the editor on the clip; selection starts empty.
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

// ---------------------------------------------------------------------
// Selection gathering: whole clip vs. explicit selection
// ---------------------------------------------------------------------

#[test]
fn quantize_with_empty_selection_targets_whole_clip() {
    let (mut app, rx) = app_with_open_clip(4);
    let _ = drain(&rx); // discard anything from opening the editor

    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::Quantize {
        grid: grid(),
        strength: 1.0,
        swing: 0.0,
        mode: QuantizeMode::StartOnly,
        quantize_ends: false,
        iterative: false,
    }));

    let cmds = drain(&rx);
    let indices = match cmds.as_slice() {
        [AudioCommand::QuantizeMidiNotes {
            clip_id,
            indices,
            mode: QuantizeMode::StartOnly,
            quantize_ends: false,
            iterative: false,
            ..
        }] if *clip_id == CLIP => indices.clone(),
        other => panic!("expected one QuantizeMidiNotes, got {other:?}"),
    };
    // Empty selection ⇒ every note in the clip.
    assert_eq!(indices, vec![0, 1, 2, 3]);
}

#[test]
fn quantize_with_selection_targets_only_selected_in_range() {
    let (mut app, rx) = app_with_open_clip(4);
    // Select notes 2 and 9 (9 is out of range and must be dropped).
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::SelectNotesInRect {
        indices: vec![2, 9],
        additive: false,
    }));
    let _ = drain(&rx);

    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::Quantize {
        grid: grid(),
        strength: 0.5,
        swing: 0.0,
        mode: QuantizeMode::StartAndLength,
        quantize_ends: true,
        iterative: true,
    }));

    let cmds = drain(&rx);
    match cmds.as_slice() {
        [AudioCommand::QuantizeMidiNotes {
            clip_id,
            indices,
            mode: QuantizeMode::StartAndLength,
            quantize_ends: true,
            iterative: true,
            ..
        }] if *clip_id == CLIP => assert_eq!(*indices, vec![2]),
        other => panic!("expected one QuantizeMidiNotes for note 2, got {other:?}"),
    }
}

#[test]
fn bulk_op_with_no_open_editor_is_a_noop() {
    let (mut app, _task) = Resonance::new();
    let rx = app.test_capture_engine();
    // No clip pushed, no editor opened.
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::Quantize {
        grid: grid(),
        strength: 1.0,
        swing: 0.0,
        mode: QuantizeMode::StartOnly,
        quantize_ends: false,
        iterative: false,
    }));
    assert!(drain(&rx).is_empty(), "no editor open ⇒ no command");
}

#[test]
fn bulk_op_on_empty_clip_is_a_noop() {
    let (mut app, rx) = app_with_open_clip(0);
    let _ = drain(&rx);
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::Quantize {
        grid: grid(),
        strength: 1.0,
        swing: 0.0,
        mode: QuantizeMode::StartOnly,
        quantize_ends: false,
        iterative: false,
    }));
    assert!(drain(&rx).is_empty(), "empty clip ⇒ no command");
}

// ---------------------------------------------------------------------
// Humanize: seed handling
// ---------------------------------------------------------------------

#[test]
fn humanize_passes_pinned_seed_through() {
    let (mut app, rx) = app_with_open_clip(3);
    let _ = drain(&rx);
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::Humanize {
        timing: 20,
        vel: 0.1,
        seed: Some(0xABCD),
    }));
    let cmds = drain(&rx);
    match cmds.as_slice() {
        [AudioCommand::HumanizeMidiNotes {
            clip_id,
            indices,
            timing_ticks,
            vel_amt,
            seed,
        }] if *clip_id == CLIP => {
            assert_eq!(*indices, vec![0, 1, 2]);
            assert_eq!(*timing_ticks, 20);
            assert!((*vel_amt - 0.1).abs() < 1e-6);
            assert_eq!(*seed, 0xABCD);
        }
        other => panic!("expected one HumanizeMidiNotes, got {other:?}"),
    }
}

#[test]
fn humanize_without_pinned_seed_still_emits_a_seeded_command() {
    let (mut app, rx) = app_with_open_clip(2);
    let _ = drain(&rx);
    // `None` ⇒ the handler draws one fresh seed for this invocation.
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::Humanize {
        timing: 10,
        vel: 0.05,
        seed: None,
    }));
    let cmds = drain(&rx);
    assert!(
        matches!(
            cmds.as_slice(),
            [AudioCommand::HumanizeMidiNotes { clip_id, .. }] if *clip_id == CLIP
        ),
        "expected one HumanizeMidiNotes, got {cmds:?}",
    );
}

// ---------------------------------------------------------------------
// Groove apply / extract
// ---------------------------------------------------------------------

#[test]
fn apply_known_stock_groove_emits_command_with_template() {
    let (mut app, rx) = app_with_open_clip(4);
    let _ = drain(&rx);
    let (name, expected) = stock_grooves().into_iter().next().expect("a stock groove");

    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::ApplyGroove {
        template_id: name,
        strength: 0.75,
    }));

    let cmds = drain(&rx);
    match cmds.as_slice() {
        [AudioCommand::ApplyGrooveToClip {
            clip_id,
            indices,
            template,
            strength,
        }] if *clip_id == CLIP => {
            assert_eq!(*indices, vec![0, 1, 2, 3]);
            assert_eq!(*template, expected);
            assert!((*strength - 0.75).abs() < 1e-6);
        }
        other => panic!("expected one ApplyGrooveToClip, got {other:?}"),
    }
}

#[test]
fn apply_unknown_groove_is_a_noop() {
    let (mut app, rx) = app_with_open_clip(4);
    let _ = drain(&rx);
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::ApplyGroove {
        template_id: "no such groove".to_string(),
        strength: 1.0,
    }));
    assert!(drain(&rx).is_empty(), "unknown template id ⇒ no command");
}

#[test]
fn extract_groove_emits_command_regardless_of_selection() {
    let (mut app, rx) = app_with_open_clip(4);
    let _ = drain(&rx);
    // Extraction reads the whole clip, so an empty selection is fine.
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::ExtractGroove {
        grid: grid(),
    }));
    let cmds = drain(&rx);
    match cmds.as_slice() {
        [AudioCommand::ExtractGrooveFromClip { clip_id, grid: g }] if *clip_id == CLIP => {
            assert_eq!(*g, grid());
        }
        other => panic!("expected one ExtractGrooveFromClip, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Undo classification: note-mutating ops record one step; extraction skips
// ---------------------------------------------------------------------

#[test]
fn note_mutating_ops_record_a_single_undo_step() {
    for msg in [
        MidiEditorMessage::Quantize {
            grid: grid(),
            strength: 1.0,
            swing: 0.0,
            mode: QuantizeMode::StartOnly,
            quantize_ends: false,
            iterative: false,
        },
        MidiEditorMessage::Humanize {
            timing: 10,
            vel: 0.1,
            seed: None,
        },
        MidiEditorMessage::ApplyGroove {
            template_id: "MPC Swing".to_string(),
            strength: 1.0,
        },
    ] {
        assert!(
            matches!(
                classify(&Message::MidiEditor(msg.clone())),
                UndoAction::Record
            ),
            "expected Record for {msg:?}",
        );
    }
}

#[test]
fn extract_groove_is_not_undoable() {
    assert!(matches!(
        classify(&Message::MidiEditor(MidiEditorMessage::ExtractGroove {
            grid: grid()
        })),
        UndoAction::Skip
    ));
}
