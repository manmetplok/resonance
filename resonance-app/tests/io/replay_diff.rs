//! The undo restore's note-equality helper (`midi_notes_equal`): field-wise
//! comparison of MIDI notes, exercised across equal/unequal/different-length
//! slices.
//!
//! This file also held the `structurally_compatible` / `id_set_eq` shape
//! tests, which chose between the diff restore and the `ClearAll` fallback.
//! Since A-13i the check accepted every pair; A-13j deleted it with the
//! fallback. Which shapes an undo restores in place is now guarded by
//! `undo_diff_shape.rs` end to end.

use resonance_app::update::project_io::replay_diff::midi_notes_equal;
use resonance_audio::types::MidiNote;

#[test]
fn midi_notes_equal_field_wise() {
    let n = |note, vel, start, dur| MidiNote {
        note,
        velocity: vel,
        start_tick: start,
        duration_ticks: dur,
    };
    assert!(midi_notes_equal(&[], &[]));
    assert!(midi_notes_equal(
        &[n(60, 0.8, 0, 480)],
        &[n(60, 0.8, 0, 480)]
    ));
    assert!(!midi_notes_equal(
        &[n(60, 0.8, 0, 480)],
        &[n(62, 0.8, 0, 480)]
    ));
    assert!(!midi_notes_equal(
        &[n(60, 0.8, 0, 480)],
        &[n(60, 0.8, 0, 481)]
    ));
    // Different lengths are unequal.
    assert!(!midi_notes_equal(&[n(60, 0.8, 0, 480)], &[]));
}
