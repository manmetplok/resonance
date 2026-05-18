use resonance_music_theory::pitch::PitchClass;

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
