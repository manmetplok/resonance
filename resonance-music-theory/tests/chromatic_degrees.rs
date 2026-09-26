//! LIB-02: roman-numeral degrees in a chromatic key resolve against the
//! major (ionian) parent on the same root, not against the 12-note table.

use resonance_music_theory::chord::{Chord, ChordQuality};
use resonance_music_theory::generator::Degree;
use resonance_music_theory::pitch::PitchClass;
use resonance_music_theory::progression::{diatonic_chord, diatonic_triads};
use resonance_music_theory::scale::{Mode, Scale};

#[test]
fn chromatic_degree_table_is_the_major_parent() {
    assert_eq!(Mode::Chromatic.degree_intervals(), Mode::Major.degree_intervals());
    for mode in Mode::ALL {
        if mode != Mode::Chromatic {
            assert_eq!(&mode.degree_intervals()[..], mode.intervals());
        }
    }
}

#[test]
fn chromatic_diatonic_chords_match_major() {
    let chromatic = Scale::new(PitchClass::C, Mode::Chromatic);
    let major = Scale::new(PitchClass::C, Mode::Major);
    assert_eq!(diatonic_chord(chromatic, 1, false), Chord::new(PitchClass::C, ChordQuality::Maj));
    assert_eq!(diatonic_chord(chromatic, 4, false), Chord::new(PitchClass::F, ChordQuality::Maj));
    assert_eq!(diatonic_chord(chromatic, 5, false), Chord::new(PitchClass::G, ChordQuality::Maj));
    assert_eq!(diatonic_chord(chromatic, 5, true), Chord::new(PitchClass::G, ChordQuality::Dom7));
    assert_eq!(diatonic_triads(chromatic), diatonic_triads(major));
}

#[test]
fn chromatic_degree_to_chord_roots_match_major() {
    let s = Scale::new(PitchClass::D, Mode::Chromatic);
    assert_eq!(Degree::I.to_chord(s).root, PitchClass::D);
    assert_eq!(Degree::IV.to_chord(s).root, PitchClass::G);
    assert_eq!(Degree::V.to_chord(s).root, PitchClass::A);
    assert_eq!(Degree::FLAT_VII.to_chord(s).root, PitchClass::C);
}
