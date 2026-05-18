use resonance_music_theory::chord::{Chord, ChordQuality};
use resonance_music_theory::pitch::PitchClass;
use resonance_music_theory::progression::*;
use resonance_music_theory::scale::{Mode, Scale};

#[test]
fn c_major_diatonic_triads_are_classical() {
    let scale = Scale::new(PitchClass::C, Mode::Major);
    let triads = diatonic_triads(scale);
    // I - ii - iii - IV - V - vi - vii°
    assert_eq!(triads[0], Chord::new(PitchClass::C, ChordQuality::Maj));
    assert_eq!(triads[1], Chord::new(PitchClass::D, ChordQuality::Min));
    assert_eq!(triads[2], Chord::new(PitchClass::E, ChordQuality::Min));
    assert_eq!(triads[3], Chord::new(PitchClass::F, ChordQuality::Maj));
    assert_eq!(triads[4], Chord::new(PitchClass::G, ChordQuality::Maj));
    assert_eq!(triads[5], Chord::new(PitchClass::A, ChordQuality::Min));
    assert_eq!(triads[6], Chord::new(PitchClass::B, ChordQuality::Dim));
}

#[test]
fn a_minor_diatonic_triads_are_classical() {
    let scale = Scale::new(PitchClass::A, Mode::Minor);
    let triads = diatonic_triads(scale);
    // i - ii° - III - iv - v - VI - VII
    assert_eq!(triads[0], Chord::new(PitchClass::A, ChordQuality::Min));
    assert_eq!(triads[1], Chord::new(PitchClass::B, ChordQuality::Dim));
    assert_eq!(triads[2], Chord::new(PitchClass::C, ChordQuality::Maj));
    assert_eq!(triads[3], Chord::new(PitchClass::D, ChordQuality::Min));
    assert_eq!(triads[4], Chord::new(PitchClass::E, ChordQuality::Min));
    assert_eq!(triads[5], Chord::new(PitchClass::F, ChordQuality::Maj));
    assert_eq!(triads[6], Chord::new(PitchClass::G, ChordQuality::Maj));
}

#[test]
fn c_major_diatonic_sevenths() {
    let scale = Scale::new(PitchClass::C, Mode::Major);
    assert_eq!(
        diatonic_chord(scale, 1, true),
        Chord::new(PitchClass::C, ChordQuality::Maj7)
    );
    assert_eq!(
        diatonic_chord(scale, 2, true),
        Chord::new(PitchClass::D, ChordQuality::Min7)
    );
    assert_eq!(
        diatonic_chord(scale, 5, true),
        Chord::new(PitchClass::G, ChordQuality::Dom7)
    );
    assert_eq!(
        diatonic_chord(scale, 7, true),
        Chord::new(PitchClass::B, ChordQuality::HalfDim7)
    );
}

#[test]
fn dorian_diatonic_triads() {
    // D Dorian = D E F G A B C — vi° at (scale_degree 6) is B diminished?
    // No: Dorian intervals are [0,2,3,5,7,9,10]; the built triads are:
    // i (D-F-A) min, ii (E-G-B) min, III (F-A-C) maj, IV (G-B-D) maj,
    // v (A-C-E) min, vi° (B-D-F) dim, VII (C-E-G) maj.
    let scale = Scale::new(PitchClass::D, Mode::Dorian);
    let triads = diatonic_triads(scale);
    assert_eq!(triads[0], Chord::new(PitchClass::D, ChordQuality::Min));
    assert_eq!(triads[1], Chord::new(PitchClass::E, ChordQuality::Min));
    assert_eq!(triads[2], Chord::new(PitchClass::F, ChordQuality::Maj));
    assert_eq!(triads[3], Chord::new(PitchClass::G, ChordQuality::Maj));
    assert_eq!(triads[4], Chord::new(PitchClass::A, ChordQuality::Min));
    assert_eq!(triads[5], Chord::new(PitchClass::B, ChordQuality::Dim));
    assert_eq!(triads[6], Chord::new(PitchClass::C, ChordQuality::Maj));
}

#[test]
fn degree_function_classification() {
    assert_eq!(degree_function(1), Function::Tonic);
    assert_eq!(degree_function(2), Function::Subdominant);
    assert_eq!(degree_function(3), Function::Tonic);
    assert_eq!(degree_function(4), Function::Subdominant);
    assert_eq!(degree_function(5), Function::Dominant);
    assert_eq!(degree_function(6), Function::Tonic);
    assert_eq!(degree_function(7), Function::Dominant);
}

#[test]
fn transitions_sum_to_one() {
    for row in &TRANSITIONS {
        let sum: f32 = row.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5, "row {row:?} sums to {sum}");
    }
}

#[test]
fn progression_starts_and_ends_on_tonic() {
    let scale = Scale::new(PitchClass::C, Mode::Major);
    for seed in 0..50 {
        let p = ProgressionParams {
            scale,
            chord_count: 4,
            seventh_chords: false,
            seed,
        };
        let chords = walk_progression(&p);
        assert_eq!(chords.len(), 4);
        assert_eq!(chords[0].root, PitchClass::C, "seed {seed} first chord");
        assert_eq!(
            chords.last().unwrap().root,
            PitchClass::C,
            "seed {seed} last chord"
        );
    }
}

#[test]
fn progression_contains_only_diatonic_chords() {
    let scale = Scale::new(PitchClass::A, Mode::Minor);
    let diatonic = diatonic_triads(scale);
    let p = ProgressionParams {
        scale,
        chord_count: 8,
        seventh_chords: false,
        seed: 42,
    };
    let chords = walk_progression(&p);
    for c in &chords {
        assert!(
            diatonic.iter().any(|d| d == c),
            "non-diatonic chord {c:?} in A-minor walk"
        );
    }
}

#[test]
fn same_seed_same_result() {
    let scale = Scale::new(PitchClass::G, Mode::Mixolydian);
    let mk = || ProgressionParams {
        scale,
        chord_count: 6,
        seventh_chords: true,
        seed: 12345,
    };
    assert_eq!(walk_progression(&mk()), walk_progression(&mk()));
}

#[test]
fn single_chord_progression_is_tonic() {
    let scale = Scale::new(PitchClass::C, Mode::Major);
    let p = ProgressionParams {
        scale,
        chord_count: 1,
        seventh_chords: false,
        seed: 0,
    };
    let chords = walk_progression(&p);
    assert_eq!(chords.len(), 1);
    assert_eq!(chords[0].root, PitchClass::C);
}

#[test]
fn b_minor_diatonic_triads() {
    let scale = Scale::new(PitchClass::B, Mode::Minor);
    let triads = diatonic_triads(scale);
    // i - ii° - III - iv - v - VI - VII
    assert_eq!(triads[0], Chord::new(PitchClass::B, ChordQuality::Min));
    assert_eq!(triads[1], Chord::new(PitchClass::Cs, ChordQuality::Dim));
    assert_eq!(triads[2], Chord::new(PitchClass::D, ChordQuality::Maj));
    assert_eq!(triads[3], Chord::new(PitchClass::E, ChordQuality::Min));
    assert_eq!(triads[4], Chord::new(PitchClass::Fs, ChordQuality::Min));
    assert_eq!(triads[5], Chord::new(PitchClass::G, ChordQuality::Maj));
    assert_eq!(triads[6], Chord::new(PitchClass::A, ChordQuality::Maj));
}

#[test]
fn b_minor_degree_1_is_minor() {
    let scale = Scale::new(PitchClass::B, Mode::Minor);
    let chord = diatonic_chord(scale, 1, false);
    assert_eq!(chord, Chord::new(PitchClass::B, ChordQuality::Min));
}

#[test]
fn b_minor_degree_1_seventh_is_minor7() {
    let scale = Scale::new(PitchClass::B, Mode::Minor);
    let chord = diatonic_chord(scale, 1, true);
    assert_eq!(chord, Chord::new(PitchClass::B, ChordQuality::Min7));
}

#[test]
fn minor_scale_walk_tonic_is_minor() {
    // For any minor-scale walk, the first and last chords (tonic) must
    // be minor — never major.
    for root in [PitchClass::A, PitchClass::B, PitchClass::D, PitchClass::E] {
        let scale = Scale::new(root, Mode::Minor);
        for seed in 0..20 {
            let p = ProgressionParams {
                scale,
                chord_count: 4,
                seventh_chords: false,
                seed,
            };
            let chords = walk_progression(&p);
            assert_eq!(
                chords[0].quality,
                ChordQuality::Min,
                "{root:?} minor seed {seed}: tonic should be minor, got {:?}",
                chords[0]
            );
            assert_eq!(
                chords.last().unwrap().quality,
                ChordQuality::Min,
                "{root:?} minor seed {seed}: final should be minor, got {:?}",
                chords.last().unwrap()
            );
        }
    }
}
