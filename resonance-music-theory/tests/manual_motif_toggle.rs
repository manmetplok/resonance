//! Unit-level coverage for `toggle_manual_motif_cell`. The function
//! drives the click-to-edit semantics of the manual motif canvas, so the
//! insert/replace/remove transitions need direct tests independent of
//! the larger render pipeline.

use resonance_music_theory::{
    toggle_manual_motif_cell, ManualMotifCell, ManualMotifNote,
};

fn note(scale_step: i8, dur: u8) -> ManualMotifNote {
    ManualMotifNote {
        scale_step,
        duration_sixteenths: dur,
        accent: false,
        is_rest: false,
    }
}

fn rest(dur: u8) -> ManualMotifNote {
    ManualMotifNote {
        scale_step: 0,
        duration_sixteenths: dur,
        accent: false,
        is_rest: true,
    }
}

#[test]
fn toggle_appends_note_to_empty_motif() {
    let mut notes: Vec<ManualMotifNote> = Vec::new();
    toggle_manual_motif_cell(&mut notes, ManualMotifCell::Note { scale_step: 3 }, 0);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].scale_step, 3);
    assert_eq!(notes[0].duration_sixteenths, 1);
    assert!(!notes[0].is_rest);
}

#[test]
fn toggle_appends_past_existing_motif() {
    let mut notes = vec![note(0, 2), note(2, 2)];
    toggle_manual_motif_cell(&mut notes, ManualMotifCell::Note { scale_step: 4 }, 4);
    assert_eq!(notes.len(), 3);
    assert_eq!(notes[2].scale_step, 4);
    assert_eq!(notes[2].duration_sixteenths, 1);
}

#[test]
fn toggle_at_note_start_with_matching_kind_removes_it() {
    let mut notes = vec![note(0, 2), note(2, 2), note(4, 2)];
    toggle_manual_motif_cell(&mut notes, ManualMotifCell::Note { scale_step: 2 }, 2);
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].scale_step, 0);
    assert_eq!(notes[1].scale_step, 4);
}

#[test]
fn toggle_at_note_start_with_other_step_replaces_pitch() {
    let mut notes = vec![note(0, 2), note(2, 2)];
    toggle_manual_motif_cell(&mut notes, ManualMotifCell::Note { scale_step: 5 }, 2);
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[1].scale_step, 5);
    assert_eq!(notes[1].duration_sixteenths, 2, "duration preserved");
}

#[test]
fn toggle_inside_note_tail_replaces_pitch() {
    let mut notes = vec![note(0, 4)];
    // beat 0 = start; beat 2 is inside the tail of the 4-sixteenth note.
    toggle_manual_motif_cell(&mut notes, ManualMotifCell::Note { scale_step: 7 }, 2);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].scale_step, 7);
    assert_eq!(notes[0].duration_sixteenths, 4, "duration kept");
}

#[test]
fn toggle_rest_at_note_start_converts_note_to_rest() {
    let mut notes = vec![note(3, 2)];
    notes[0].accent = true;
    toggle_manual_motif_cell(&mut notes, ManualMotifCell::Rest, 0);
    assert_eq!(notes.len(), 1);
    assert!(notes[0].is_rest);
    assert!(!notes[0].accent, "accent cleared on rest conversion");
    assert_eq!(notes[0].duration_sixteenths, 2);
}

#[test]
fn toggle_note_at_rest_start_converts_rest_to_note() {
    let mut notes = vec![rest(2)];
    toggle_manual_motif_cell(&mut notes, ManualMotifCell::Note { scale_step: 1 }, 0);
    assert_eq!(notes.len(), 1);
    assert!(!notes[0].is_rest);
    assert_eq!(notes[0].scale_step, 1);
    assert_eq!(notes[0].duration_sixteenths, 2);
}

#[test]
fn toggle_rest_at_existing_rest_removes_it() {
    let mut notes = vec![rest(2), note(2, 2)];
    toggle_manual_motif_cell(&mut notes, ManualMotifCell::Rest, 0);
    assert_eq!(notes.len(), 1);
    assert!(!notes[0].is_rest);
    assert_eq!(notes[0].scale_step, 2);
}

#[test]
fn toggle_treats_zero_duration_as_one_sixteenth() {
    // duration_sixteenths == 0 should not cause an infinite loop or
    // misalign the cursor; it's clamped to at least 1 internally.
    let mut notes = vec![ManualMotifNote {
        scale_step: 0,
        duration_sixteenths: 0,
        accent: false,
        is_rest: false,
    }];
    toggle_manual_motif_cell(&mut notes, ManualMotifCell::Note { scale_step: 4 }, 1);
    // The append branch should fire — the cursor advanced past beat 1.
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[1].scale_step, 4);
}
