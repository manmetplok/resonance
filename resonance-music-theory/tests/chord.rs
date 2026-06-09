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
        c.pitch_classes().collect::<Vec<_>>(),
        vec![PitchClass::C, PitchClass::E, PitchClass::G]
    );
}

#[test]
fn d_minor7_pitch_classes() {
    let c = Chord::new(PitchClass::D, ChordQuality::Min7);
    assert_eq!(
        c.pitch_classes().collect::<Vec<_>>(),
        vec![PitchClass::D, PitchClass::F, PitchClass::A, PitchClass::C]
    );
}

#[test]
fn pitch_classes_iterator_matches_interval_transposition() {
    // The lazy iterator must yield exactly what the old allocating
    // implementation returned: the root transposed by each quality
    // interval, in interval order.
    for root in 0..12u8 {
        let root_pc = PitchClass::from_semitone(root);
        for q in ChordQuality::ALL {
            let chord = Chord::new(root_pc, q);
            let expected: Vec<PitchClass> = q
                .intervals()
                .iter()
                .map(|&iv| root_pc.transpose(iv as i32))
                .collect();
            assert_eq!(chord.pitch_classes().collect::<Vec<_>>(), expected, "{chord}");
            assert_eq!(chord.pitch_classes().count(), q.intervals().len(), "{chord}");
        }
    }
}
