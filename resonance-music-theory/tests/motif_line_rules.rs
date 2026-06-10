//! Well-formed-line rules for generated motifs (Open Music Theory v2):
//! total range capped at a major 10th (16 semitones), no tritone leaps,
//! no leaps wider than a perfect 5th (7 semitones), and at most two
//! identical consecutive pitches.
//!
//! Exercised through the public [`motif_intervals`] entry point, which
//! returns exactly the interval contour `build_motif` produced.

use resonance_music_theory::{
    motif_intervals, Chord, ChordQuality, Mode, MotifParams, MotifSource, PitchClass, Scale,
};

/// Assert all line rules on a motif's interval contour.
fn assert_well_formed_line(intervals: &[i8], ctx: &str) {
    assert!(!intervals.is_empty(), "empty motif for {ctx}");

    // Range rule: highest minus lowest pitch is at most a major 10th.
    let min = *intervals.iter().min().unwrap();
    let max = *intervals.iter().max().unwrap();
    assert!(
        i16::from(max) - i16::from(min) <= 16,
        "range exceeds a 10th ({min}..{max}) in {intervals:?} for {ctx}"
    );

    // Leap rules: no tritone (6 semitones), nothing past a perfect 5th.
    for pair in intervals.windows(2) {
        let leap = (i16::from(pair[1]) - i16::from(pair[0])).abs();
        assert_ne!(leap, 6, "tritone leap in {intervals:?} for {ctx}");
        assert!(
            leap <= 7,
            "leap of {leap} semitones (> perfect 5th) in {intervals:?} for {ctx}"
        );
    }

    // Repeat rule: at most two identical consecutive pitches.
    for triple in intervals.windows(3) {
        assert!(
            !(triple[0] == triple[1] && triple[1] == triple[2]),
            "more than two repeated pitches in {intervals:?} for {ctx}"
        );
    }
}

fn params(seed: u64, complexity: f32, leap_chance: f32) -> MotifParams {
    MotifParams {
        seed,
        complexity,
        motif_len: 0, // derive length from complexity
        leap_chance,
    }
}

const CHORDS: [(PitchClass, ChordQuality); 4] = [
    (PitchClass::C, ChordQuality::Maj),
    (PitchClass::A, ChordQuality::Min),
    (PitchClass::G, ChordQuality::Dom7),
    (PitchClass::B, ChordQuality::Dim),
];

#[test]
fn generated_motifs_obey_line_rules_with_scale() {
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    for seed in 0..400 {
        for &(root, quality) in &CHORDS {
            let source = MotifSource::Generated(params(seed, 0.6, 0.3));
            let intervals = motif_intervals(&source, Chord::new(root, quality), scale);
            assert_well_formed_line(
                &intervals,
                &format!("seed {seed}, chord {root:?} {quality:?}, with scale"),
            );
        }
    }
}

#[test]
fn generated_motifs_obey_line_rules_without_scale() {
    // Without a scale, every interval is snapped to a chord tone during
    // construction — snapping must not reintroduce illegal leaps.
    for seed in 0..400 {
        for &(root, quality) in &CHORDS {
            let source = MotifSource::Generated(params(seed, 0.6, 0.3));
            let intervals = motif_intervals(&source, Chord::new(root, quality), None);
            assert_well_formed_line(
                &intervals,
                &format!("seed {seed}, chord {root:?} {quality:?}, no scale"),
            );
        }
    }
}

#[test]
fn generated_motifs_obey_line_rules_across_complexity_and_leap_chance() {
    let scale = Some(Scale::new(PitchClass::D, Mode::Minor));
    let chord = Chord::new(PitchClass::D, ChordQuality::Min);
    for seed in 0..120 {
        for &complexity in &[0.0f32, 0.25, 0.5, 0.75, 1.0] {
            // Includes a leap-heavy extreme where step_chance goes to
            // zero and nearly every move is drawn as a leap.
            for &leap_chance in &[0.0f32, 0.21, 0.5, 0.89] {
                let source = MotifSource::Generated(params(seed, complexity, leap_chance));
                let intervals = motif_intervals(&source, chord, scale);
                assert_well_formed_line(
                    &intervals,
                    &format!("seed {seed}, complexity {complexity}, leap_chance {leap_chance}"),
                );
            }
        }
    }
}

#[test]
fn generated_motifs_obey_line_rules_at_fixed_lengths() {
    let scale = Some(Scale::new(PitchClass::E, Mode::Major));
    let chord = Chord::new(PitchClass::E, ChordQuality::Maj);
    for seed in 0..120 {
        for motif_len in 2..=6u8 {
            let source = MotifSource::Generated(MotifParams {
                seed,
                complexity: 0.8,
                motif_len,
                leap_chance: 0.4,
            });
            let intervals = motif_intervals(&source, chord, scale);
            assert_eq!(intervals.len(), motif_len as usize);
            assert_well_formed_line(
                &intervals,
                &format!("seed {seed}, motif_len {motif_len}"),
            );
        }
    }
}

#[test]
fn motif_generation_stays_deterministic() {
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let chord = Chord::new(PitchClass::C, ChordQuality::Maj);
    let source = MotifSource::Generated(params(42, 0.5, 0.21));
    let a = motif_intervals(&source, chord, scale);
    let b = motif_intervals(&source, chord, scale);
    assert_eq!(a, b);
}
