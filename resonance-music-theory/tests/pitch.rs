use resonance_music_theory::pitch::{midi_note_name, midi_note_name_unicode, PitchClass};

#[test]
fn semitone_roundtrip() {
    for pc in PitchClass::ALL {
        assert_eq!(PitchClass::from_semitone(pc.to_semitone()), pc);
    }
}

#[test]
fn transpose_wraps() {
    assert_eq!(PitchClass::B.transpose(1), PitchClass::C);
    assert_eq!(PitchClass::C.transpose(-1), PitchClass::B);
    assert_eq!(PitchClass::C.transpose(12), PitchClass::C);
    assert_eq!(PitchClass::G.transpose(5), PitchClass::C);
}

#[test]
fn midi_note_name_extremes() {
    assert_eq!(midi_note_name(0), "C-1");
    assert_eq!(midi_note_name(127), "G9");
}

#[test]
fn midi_note_name_middle_c_is_c4() {
    assert_eq!(midi_note_name(60), "C4");
    assert_eq!(midi_note_name(69), "A4");
}

#[test]
fn midi_note_name_octave_boundaries() {
    assert_eq!(midi_note_name(11), "B-1");
    assert_eq!(midi_note_name(12), "C0");
    assert_eq!(midi_note_name(59), "B3");
    assert_eq!(midi_note_name(61), "C#4");
}

#[test]
fn midi_note_name_unicode_sharp() {
    assert_eq!(midi_note_name_unicode(61), "C\u{266f}4");
    assert_eq!(midi_note_name_unicode(60), "C4");
}
