use resonance_music_theory::pitch::PitchClass;
use resonance_music_theory::scale::{Mode, Scale};

#[test]
fn c_major_contains_diatonic_notes() {
    let s = Scale::new(PitchClass::C, Mode::Major);
    // C D E F G A B across any octave
    for base in [0u8, 12, 24, 36, 60, 72, 84].iter() {
        assert!(s.contains(*base)); // C
        assert!(s.contains(base + 2)); // D
        assert!(s.contains(base + 4)); // E
        assert!(s.contains(base + 5)); // F
        assert!(s.contains(base + 7)); // G
        assert!(s.contains(base + 9)); // A
        assert!(s.contains(base + 11)); // B
        assert!(!s.contains(base + 1)); // C#
        assert!(!s.contains(base + 3)); // D#
    }
}

#[test]
fn d_dorian_contains_expected() {
    let s = Scale::new(PitchClass::D, Mode::Dorian);
    // D dorian = D E F G A B C = all white keys starting from D
    let d4 = 62u8;
    for iv in [0, 2, 3, 5, 7, 9, 10] {
        assert!(s.contains(d4 + iv), "D dorian should contain {}", iv);
    }
    assert!(!s.contains(d4 + 1));
    assert!(!s.contains(d4 + 4));
}

#[test]
fn display() {
    let s = Scale::new(PitchClass::C, Mode::Minor);
    assert_eq!(s.to_string(), "C minor");
    let s = Scale::new(PitchClass::D, Mode::Dorian);
    assert_eq!(s.to_string(), "D dorian");
}
