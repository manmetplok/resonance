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

// ---- Spelling in a key (BUG-72) ----

use resonance_music_theory::scale::{Mode, Scale};

#[test]
fn display_in_flat_key_spells_flats() {
    let d_minor = Scale::new(PitchClass::D, Mode::Minor);
    let c = Chord::new(PitchClass::As, ChordQuality::Maj7);
    assert_eq!(c.display_in(Some(&d_minor)).to_string(), "Bbmaj7");
    // Plain Display has no key and stays on sharps.
    assert_eq!(c.to_string(), "A#maj7");
}

#[test]
fn display_in_spells_slash_bass_in_the_key() {
    let e_flat = Scale::new(PitchClass::Ds, Mode::Major);
    let c = Chord::new(PitchClass::Gs, ChordQuality::Maj).with_bass(PitchClass::Ds);
    assert_eq!(c.display_in(Some(&e_flat)).to_string(), "Ab/Eb");
}

#[test]
fn display_in_sharp_key_or_no_key_spells_sharps() {
    let e_major = Scale::new(PitchClass::E, Mode::Major);
    let c = Chord::new(PitchClass::Cs, ChordQuality::Min7);
    assert_eq!(c.display_in(Some(&e_major)).to_string(), "C#m7");
    assert_eq!(c.display_in(None).to_string(), "C#m7");
    let c_major = Scale::new(PitchClass::C, Mode::Major);
    assert_eq!(
        Chord::new(PitchClass::As, ChordQuality::Maj)
            .display_in(Some(&c_major))
            .to_string(),
        "A#"
    );
}

#[test]
fn display_in_round_trips_through_the_parser() {
    let f_major = Scale::new(PitchClass::F, Mode::Major);
    for root in PitchClass::ALL {
        let c = Chord::new(root, ChordQuality::Dom7).with_bass(root.transpose(4));
        let shown = c.display_in(Some(&f_major)).to_string();
        assert_eq!(parse_chord(&shown).unwrap(), c, "{shown}");
    }
}

#[test]
fn prefers_flats_follows_the_key_signature() {
    use PitchClass::*;
    for (root, mode, flats) in [
        (F, Mode::Major, true),
        (As, Mode::Major, true),
        (Ds, Mode::Major, true),
        (Gs, Mode::Major, true),
        (Cs, Mode::Major, true),
        (Fs, Mode::Major, true),
        (D, Mode::Minor, true),
        (G, Mode::Minor, true),
        (C, Mode::Minor, true),
        (F, Mode::Minor, true),
        (As, Mode::Minor, true),
        (Ds, Mode::Minor, true),
        (G, Mode::Dorian, true),
        (D, Mode::HarmonicMinor, true),
        (C, Mode::Major, false),
        (G, Mode::Major, false),
        (D, Mode::Major, false),
        (A, Mode::Minor, false),
        (E, Mode::Minor, false),
        (B, Mode::Minor, false),
        (D, Mode::Dorian, false),
        (Fs, Mode::HarmonicMinor, false),
    ] {
        assert_eq!(Scale::new(root, mode).prefers_flats(), flats, "{root:?} {mode:?}");
    }
}

#[test]
fn scale_display_spells_its_own_root() {
    assert_eq!(Scale::new(PitchClass::As, Mode::Major).to_string(), "Bb major");
    assert_eq!(Scale::new(PitchClass::Fs, Mode::Minor).to_string(), "F# minor");
    assert_eq!(Scale::new(PitchClass::As, Mode::Major).root_name(), "Bb");
}
