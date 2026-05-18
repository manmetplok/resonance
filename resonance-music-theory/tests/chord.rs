use resonance_music_theory::chord::*;
use resonance_music_theory::pitch::PitchClass;

#[test]
fn display_cmaj7() {
    let c = Chord::new(PitchClass::C, ChordQuality::Maj7);
    assert_eq!(c.to_string(), "Cmaj7");
}

#[test]
fn display_slash() {
    let c = Chord::new(PitchClass::G, ChordQuality::Maj).with_bass(PitchClass::B);
    assert_eq!(c.to_string(), "G/B");
}

#[test]
fn display_minor() {
    let c = Chord::new(PitchClass::A, ChordQuality::Min);
    assert_eq!(c.to_string(), "Am");
}

#[test]
fn c_major_pitch_classes() {
    let c = Chord::new(PitchClass::C, ChordQuality::Maj);
    assert_eq!(
        c.pitch_classes(),
        vec![PitchClass::C, PitchClass::E, PitchClass::G]
    );
}

#[test]
fn d_minor7_pitch_classes() {
    let c = Chord::new(PitchClass::D, ChordQuality::Min7);
    assert_eq!(
        c.pitch_classes(),
        vec![PitchClass::D, PitchClass::F, PitchClass::A, PitchClass::C]
    );
}
