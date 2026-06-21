use resonance_music_theory::chord::*;
use resonance_music_theory::pitch::PitchClass;

fn parse(s: &str) -> Chord {
    parse_chord(s).unwrap_or_else(|e| panic!("failed to parse {s:?}: {e}"))
}

#[test]
fn parses_bare_major_triad() {
    assert_eq!(parse("C"), Chord::new(PitchClass::C, ChordQuality::Maj));
    assert_eq!(parse("G"), Chord::new(PitchClass::G, ChordQuality::Maj));
}

#[test]
fn parses_canonical_suffixes() {
    assert_eq!(parse("Cm"), Chord::new(PitchClass::C, ChordQuality::Min));
    assert_eq!(parse("Cdim"), Chord::new(PitchClass::C, ChordQuality::Dim));
    assert_eq!(parse("Caug"), Chord::new(PitchClass::C, ChordQuality::Aug));
    assert_eq!(parse("Cmaj7"), Chord::new(PitchClass::C, ChordQuality::Maj7));
    assert_eq!(parse("Cm7"), Chord::new(PitchClass::C, ChordQuality::Min7));
    assert_eq!(parse("C7"), Chord::new(PitchClass::C, ChordQuality::Dom7));
    assert_eq!(parse("CmMaj7"), Chord::new(PitchClass::C, ChordQuality::MinMaj7));
    assert_eq!(parse("Cdim7"), Chord::new(PitchClass::C, ChordQuality::Dim7));
    assert_eq!(parse("Cm7b5"), Chord::new(PitchClass::C, ChordQuality::HalfDim7));
    assert_eq!(parse("Csus2"), Chord::new(PitchClass::C, ChordQuality::Sus2));
    assert_eq!(parse("Csus4"), Chord::new(PitchClass::C, ChordQuality::Sus4));
    assert_eq!(parse("C6"), Chord::new(PitchClass::C, ChordQuality::Maj6));
    assert_eq!(parse("Cm6"), Chord::new(PitchClass::C, ChordQuality::Min6));
    assert_eq!(parse("Cadd9"), Chord::new(PitchClass::C, ChordQuality::Add9));
}

/// The headline guarantee: every quality round-trips through Display for
/// every root, with and without a slash bass.
#[test]
fn round_trips_display_for_every_quality_and_root() {
    for root in PitchClass::ALL {
        for q in ChordQuality::ALL {
            let chord = Chord::new(root, q);
            let rendered = chord.to_string();
            assert_eq!(parse(&rendered), chord, "round-trip failed for {rendered:?}");

            let with_bass = chord.with_bass(PitchClass::E);
            let rendered = with_bass.to_string();
            assert_eq!(
                parse(&rendered),
                with_bass,
                "slash round-trip failed for {rendered:?}"
            );
        }
    }
}

#[test]
fn from_str_matches_parse_chord() {
    let chord: Chord = "F#m7".parse().unwrap();
    assert_eq!(chord, Chord::new(PitchClass::Fs, ChordQuality::Min7));
}

#[test]
fn parses_sharp_root() {
    assert_eq!(parse("C#"), Chord::new(PitchClass::Cs, ChordQuality::Maj));
    assert_eq!(parse("F#m"), Chord::new(PitchClass::Fs, ChordQuality::Min));
}

#[test]
fn parses_flat_root_to_enharmonic() {
    // PitchClass has no flat spellings; Db == C#, Bb == A#.
    assert_eq!(parse("Db"), Chord::new(PitchClass::Cs, ChordQuality::Maj));
    assert_eq!(parse("Bb7"), Chord::new(PitchClass::As, ChordQuality::Dom7));
    assert_eq!(parse("Eb"), Chord::new(PitchClass::Ds, ChordQuality::Maj));
}

#[test]
fn parses_double_accidentals() {
    // F## == G, Dbb == C.
    assert_eq!(parse("F##"), Chord::new(PitchClass::G, ChordQuality::Maj));
    assert_eq!(parse("Dbb"), Chord::new(PitchClass::C, ChordQuality::Maj));
    // 'x' is a double sharp alias.
    assert_eq!(parse("Fx"), Chord::new(PitchClass::G, ChordQuality::Maj));
}

#[test]
fn parses_unicode_accidentals() {
    assert_eq!(parse("C\u{266f}"), Chord::new(PitchClass::Cs, ChordQuality::Maj));
    assert_eq!(parse("D\u{266d}"), Chord::new(PitchClass::Cs, ChordQuality::Maj));
}

#[test]
fn parses_slash_bass() {
    assert_eq!(
        parse("G/B"),
        Chord::new(PitchClass::G, ChordQuality::Maj).with_bass(PitchClass::B)
    );
    assert_eq!(
        parse("D7/F#"),
        Chord::new(PitchClass::D, ChordQuality::Dom7).with_bass(PitchClass::Fs)
    );
    assert_eq!(
        parse("C/Bb"),
        Chord::new(PitchClass::C, ChordQuality::Maj).with_bass(PitchClass::As)
    );
}

#[test]
fn parses_common_aliases() {
    assert_eq!(parse("C-"), Chord::new(PitchClass::C, ChordQuality::Min));
    assert_eq!(parse("Cmin"), Chord::new(PitchClass::C, ChordQuality::Min));
    assert_eq!(parse("C+"), Chord::new(PitchClass::C, ChordQuality::Aug));
    assert_eq!(parse("C\u{394}"), Chord::new(PitchClass::C, ChordQuality::Maj7));
    assert_eq!(parse("C\u{b0}"), Chord::new(PitchClass::C, ChordQuality::Dim));
    assert_eq!(parse("C\u{b0}7"), Chord::new(PitchClass::C, ChordQuality::Dim7));
    assert_eq!(parse("C\u{f8}"), Chord::new(PitchClass::C, ChordQuality::HalfDim7));
    assert_eq!(parse("Csus"), Chord::new(PitchClass::C, ChordQuality::Sus4));
    assert_eq!(parse("CM"), Chord::new(PitchClass::C, ChordQuality::Maj));
}

#[test]
fn trims_surrounding_whitespace() {
    assert_eq!(parse("  Am7  "), Chord::new(PitchClass::A, ChordQuality::Min7));
}

#[test]
fn case_sensitive_major_vs_minor() {
    // Capital M is major, lower-case m is minor.
    assert_eq!(parse("CM7"), Chord::new(PitchClass::C, ChordQuality::Maj7));
    assert_eq!(parse("Cm7"), Chord::new(PitchClass::C, ChordQuality::Min7));
}

#[test]
fn rejects_empty_input() {
    assert_eq!(parse_chord(""), Err(ChordParseError::Empty));
    assert_eq!(parse_chord("   "), Err(ChordParseError::Empty));
}

#[test]
fn rejects_bad_root() {
    assert!(matches!(parse_chord("H"), Err(ChordParseError::BadRoot(_))));
    assert!(matches!(parse_chord("7"), Err(ChordParseError::BadRoot(_))));
}

#[test]
fn rejects_unknown_quality() {
    assert!(matches!(
        parse_chord("Cwobble"),
        Err(ChordParseError::BadQuality(_))
    ));
    assert!(matches!(
        parse_chord("Cmaj7xyz"),
        Err(ChordParseError::BadQuality(_))
    ));
}

#[test]
fn rejects_bad_slash_bass() {
    assert!(matches!(parse_chord("C/H"), Err(ChordParseError::BadBass(_))));
    assert!(matches!(parse_chord("C/"), Err(ChordParseError::BadBass(_))));
    assert!(matches!(
        parse_chord("C/Bm"),
        Err(ChordParseError::BadBass(_))
    ));
}

#[test]
fn error_messages_are_descriptive() {
    let err = parse_chord("Cwobble").unwrap_err();
    assert!(err.to_string().contains("wobble"), "{err}");
}
