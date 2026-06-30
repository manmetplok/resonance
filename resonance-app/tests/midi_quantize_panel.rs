//! Quantize panel control wiring (ba todo #392, doc #163, epic #25).
//!
//! The panel's controls dispatch `SetQuantize*` messages that write the
//! app-level [`MidiQuantizePanelState`]; the Apply button then reads those
//! settings to build the bulk `Quantize` message exercised by #391. These
//! tests cover the bound-state plumbing: the `GridChoice` → `Division`
//! mapping that drives the grid picker, the setter handlers (including
//! clamping), and the undo classification that keeps pure control edits
//! out of the undo history.

use resonance_app::message::{Message, MidiEditorMessage};
use resonance_app::state::GridChoice;
use resonance_app::undo::{classify, UndoAction};
use resonance_app::Resonance;
use resonance_audio::quantize::{GridModifier, GridValue, QuantizeMode};

fn dispatch(app: &mut Resonance, m: MidiEditorMessage) {
    app.test_dispatch(Message::MidiEditor(m));
}

// ---------------------------------------------------------------------
// GridChoice → Division mapping (drives the grid pick_list)
// ---------------------------------------------------------------------

#[test]
fn grid_choices_cover_quarter_to_thirtysecond_in_three_flavours() {
    // Twelve choices: 1/4 .. 1/32 each straight / triplet / dotted.
    assert_eq!(GridChoice::ALL.len(), 12);

    // No duplicate labels, and labels are stable.
    let labels: Vec<&str> = GridChoice::ALL.iter().map(|g| g.label()).collect();
    let mut sorted = labels.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), labels.len(), "labels must be unique");
    assert!(labels.contains(&"1/16"));
    assert!(labels.contains(&"1/8T"));
    assert!(labels.contains(&"1/4."));
}

#[test]
fn grid_choice_maps_to_expected_division_and_ticks() {
    let q = GridChoice::Quarter.division();
    assert_eq!(q.value, GridValue::Quarter);
    assert_eq!(q.modifier, GridModifier::Straight);
    assert_eq!(q.ticks(), 480);

    // Eighth triplet = 240 * 2 / 3.
    let et = GridChoice::EighthTriplet.division();
    assert_eq!(et.value, GridValue::Eighth);
    assert_eq!(et.modifier, GridModifier::Triplet);
    assert_eq!(et.ticks(), 160);

    // Sixteenth dotted = 120 * 3 / 2.
    let sd = GridChoice::SixteenthDotted.division();
    assert_eq!(sd.modifier, GridModifier::Dotted);
    assert_eq!(sd.ticks(), 180);

    assert_eq!(GridChoice::ThirtySecond.division().ticks(), 60);
}

// ---------------------------------------------------------------------
// Setter handlers write the bound state
// ---------------------------------------------------------------------

#[test]
fn default_panel_state_is_sensible() {
    let (app, _task) = Resonance::new();
    let s = app.test_quantize_panel();
    assert_eq!(s.grid, GridChoice::Sixteenth);
    assert_eq!(s.strength, 1.0);
    assert_eq!(s.swing, 0.0);
    assert_eq!(s.mode, QuantizeMode::StartOnly);
    assert!(!s.quantize_ends);
    assert!(!s.iterative);
}

#[test]
fn setters_update_each_bound_field() {
    let (mut app, _task) = Resonance::new();

    dispatch(&mut app, MidiEditorMessage::SetQuantizeGrid(GridChoice::EighthTriplet));
    dispatch(&mut app, MidiEditorMessage::SetQuantizeStrength(0.5));
    dispatch(&mut app, MidiEditorMessage::SetQuantizeSwing(0.25));
    dispatch(
        &mut app,
        MidiEditorMessage::SetQuantizeMode(QuantizeMode::StartAndLength),
    );
    dispatch(&mut app, MidiEditorMessage::SetQuantizeEnds(true));
    dispatch(&mut app, MidiEditorMessage::SetQuantizeIterative(true));

    let s = app.test_quantize_panel();
    assert_eq!(s.grid, GridChoice::EighthTriplet);
    assert_eq!(s.strength, 0.5);
    assert_eq!(s.swing, 0.25);
    assert_eq!(s.mode, QuantizeMode::StartAndLength);
    assert!(s.quantize_ends);
    assert!(s.iterative);
}

#[test]
fn strength_and_swing_are_clamped_to_unit_range() {
    let (mut app, _task) = Resonance::new();

    dispatch(&mut app, MidiEditorMessage::SetQuantizeStrength(5.0));
    dispatch(&mut app, MidiEditorMessage::SetQuantizeSwing(-2.0));
    let s = app.test_quantize_panel();
    assert_eq!(s.strength, 1.0);
    assert_eq!(s.swing, 0.0);
}

// ---------------------------------------------------------------------
// Undo: control edits are not undoable; Apply (Quantize) is
// ---------------------------------------------------------------------

#[test]
fn panel_control_edits_skip_undo() {
    for m in [
        MidiEditorMessage::SetQuantizeGrid(GridChoice::Quarter),
        MidiEditorMessage::SetQuantizeStrength(0.3),
        MidiEditorMessage::SetQuantizeSwing(0.3),
        MidiEditorMessage::SetQuantizeMode(QuantizeMode::StartAndLength),
        MidiEditorMessage::SetQuantizeEnds(true),
        MidiEditorMessage::SetQuantizeIterative(true),
    ] {
        assert!(
            matches!(classify(&Message::MidiEditor(m)), UndoAction::Skip),
            "quantize-panel control edits are pure view state",
        );
    }
}

#[test]
fn apply_quantize_records_one_undo_step() {
    // The Apply button dispatches the bulk Quantize message; it must be
    // a single undo step (the note edit), unlike the control setters.
    let m = MidiEditorMessage::Quantize {
        grid: GridChoice::Sixteenth.division(),
        strength: 1.0,
        swing: 0.0,
        mode: QuantizeMode::StartOnly,
        quantize_ends: false,
        iterative: false,
    };
    assert!(matches!(
        classify(&Message::MidiEditor(m)),
        UndoAction::Record
    ));
}
