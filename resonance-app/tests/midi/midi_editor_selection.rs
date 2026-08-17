//! Multi-note selection-set logic for the MIDI piano roll editor
//! (todo #389). Covers the `MidiEditorState` selection helpers that the
//! piano-roll messages drive: single-select/clear, shift/ctrl toggle,
//! marquee union/replace, select-all and the single-note representative
//! the vocal roll still reads.

use std::collections::BTreeSet;

use resonance_app::state::MidiEditorState;

fn editor() -> MidiEditorState {
    MidiEditorState {
        clip_id: 1,
        track_id: 1,
        scroll_y: 0.0,
        zoom_x: 1.0,
        zoom_y: 10.0,
        snap_ticks: 120,
        selected_notes: BTreeSet::new(),
    }
}

#[test]
fn select_single_replaces_then_clears() {
    let mut e = editor();
    e.select_single(Some(3));
    assert_eq!(e.selected_notes, BTreeSet::from([3]));
    e.select_single(Some(5));
    assert_eq!(e.selected_notes, BTreeSet::from([5]));
    e.select_single(None);
    assert!(e.selected_notes.is_empty());
}

#[test]
fn toggle_adds_then_removes() {
    let mut e = editor();
    e.toggle_note(2);
    e.toggle_note(7);
    assert_eq!(e.selected_notes, BTreeSet::from([2, 7]));
    e.toggle_note(2);
    assert_eq!(e.selected_notes, BTreeSet::from([7]));
}

#[test]
fn marquee_replaces_unless_additive() {
    let mut e = editor();
    e.apply_marquee([1, 2, 3], false);
    assert_eq!(e.selected_notes, BTreeSet::from([1, 2, 3]));
    // Additive marquee unions with the existing selection.
    e.apply_marquee([3, 8], true);
    assert_eq!(e.selected_notes, BTreeSet::from([1, 2, 3, 8]));
    // Non-additive marquee replaces it.
    e.apply_marquee([9], false);
    assert_eq!(e.selected_notes, BTreeSet::from([9]));
}

#[test]
fn select_all_then_clear() {
    let mut e = editor();
    e.select_all(4);
    assert_eq!(e.selected_notes, BTreeSet::from([0, 1, 2, 3]));
    assert!(e.is_selected(0) && e.is_selected(3));
    e.clear_selection();
    assert!(e.selected_notes.is_empty());
}

#[test]
fn primary_selected_is_lowest_index() {
    let mut e = editor();
    assert_eq!(e.primary_selected(), None);
    e.toggle_note(9);
    e.toggle_note(4);
    e.toggle_note(6);
    assert_eq!(e.primary_selected(), Some(4));
}
