//! Single-climax rule for realized motif phrases (Open Music Theory
//! v2, well-formed melodic lines): exactly one highest note per
//! phrase, placed in the phrase's second half and never on the final
//! note, ideally approached by leap. Composes with the leap grammar
//! (see tests/leap_recovery.rs) — both run on the same generator
//! output, so the climax pass must not undo a leap repair and vice
//! versa.
//!
//! Exercised through the public [`derive_motif_melody_with_section`]
//! entry point with a single phrase covering all chords, so phrase
//! joins and per-phrase octave displacement don't blur the rules.

use resonance_music_theory::{
    derive_motif_melody_with_section, Chord, ChordQuality, ContourPreference, GeneratedNote,
    MelodyParams, MelodyStyle, Mode, MotifParams, MotifSource, PitchClass, Scale, TimedChord,
};

/// Assert the single-climax rule on one phrase. Mirrors the skip
/// conditions of the enforcement pass: phrases shorter than 3 notes
/// can't host a non-final second-half climax, and flat lines have no
/// contour to discipline.
fn assert_single_climax(notes: &[GeneratedNote], ctx: &str) {
    let pitches: Vec<u8> = notes.iter().map(|n| n.note).collect();
    let n = pitches.len();
    if n < 3 {
        return;
    }
    let max = *pitches.iter().max().unwrap();
    let min = *pitches.iter().min().unwrap();
    if max == min {
        return; // flat recitation: exempt
    }
    let peaks: Vec<usize> = pitches
        .iter()
        .enumerate()
        .filter(|(_, &p)| p == max)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        peaks.len(),
        1,
        "expected exactly one climax, found peaks at {peaks:?} in {pitches:?} for {ctx}"
    );
    let climax = peaks[0];
    assert!(
        climax >= n / 2,
        "climax at {climax} sits in the first half of {n} notes in {pitches:?} for {ctx}"
    );
    assert_ne!(
        climax,
        n - 1,
        "climax must not be the final note in {pitches:?} for {ctx}"
    );
}

fn single_phrase_chords() -> Vec<TimedChord> {
    let seq = [
        (PitchClass::C, ChordQuality::Maj),
        (PitchClass::A, ChordQuality::Min),
        (PitchClass::F, ChordQuality::Maj),
        (PitchClass::G, ChordQuality::Maj),
    ];
    seq.iter()
        .enumerate()
        .map(|(i, &(root, quality))| TimedChord {
            chord: Chord::new(root, quality),
            start_beat: (i * 4) as u32,
            duration_beats: 4,
        })
        .collect()
}

fn melody_params(leap_chance: f32, complexity: f32, contour: ContourPreference) -> MelodyParams {
    MelodyParams {
        style: MelodyStyle::Motif,
        // One phrase spans all 4 chords: no joins, no octave shifts.
        phrase_len: 4,
        rest_density: 0.0,
        complexity,
        leap_chance,
        contour,
        ..MelodyParams::default()
    }
}

fn generate(
    seed: u64,
    leap_chance: f32,
    complexity: f32,
    contour: ContourPreference,
) -> Vec<GeneratedNote> {
    let chords = single_phrase_chords();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let params = melody_params(leap_chance, complexity, contour);
    let source = MotifSource::Generated(MotifParams {
        seed,
        complexity,
        motif_len: 0,
        leap_chance,
    });
    derive_motif_melody_with_section(&chords, scale, &params, &source, seed, 480)
}

#[test]
fn realized_phrases_have_a_single_second_half_climax() {
    for seed in 0..400 {
        let notes = generate(seed, 0.3, 0.6, ContourPreference::Auto);
        assert!(!notes.is_empty(), "empty melody for seed {seed}");
        assert_single_climax(&notes, &format!("seed {seed}, leap_chance 0.3"));
    }
}

#[test]
fn wave_contour_no_longer_produces_two_peaks() {
    // The wave contour traces one full sine cycle per phrase — before
    // climax enforcement, its two crests regularly produced duplicate
    // phrase peaks.
    for seed in 0..400 {
        let notes = generate(seed, 0.3, 0.6, ContourPreference::Wave);
        assert!(!notes.is_empty(), "empty melody for seed {seed}");
        assert_single_climax(&notes, &format!("seed {seed}, wave contour"));
    }
}

#[test]
fn descending_and_arch_contours_obey_the_climax_rule() {
    for &contour in &[
        ContourPreference::Descending,
        ContourPreference::Arch,
        ContourPreference::Ascending,
    ] {
        for seed in 0..200 {
            let notes = generate(seed, 0.5, 0.7, contour);
            assert!(!notes.is_empty(), "empty melody for seed {seed}");
            assert_single_climax(&notes, &format!("seed {seed}, contour {contour:?}"));
        }
    }
}

#[test]
fn leap_heavy_phrases_obey_the_climax_rule() {
    for seed in 0..400 {
        let notes = generate(seed, 0.89, 0.9, ContourPreference::Auto);
        assert!(!notes.is_empty(), "empty melody for seed {seed}");
        assert_single_climax(&notes, &format!("seed {seed}, leap_chance 0.89"));
    }
}

#[test]
fn minor_scale_phrases_obey_the_climax_rule() {
    let chords: Vec<TimedChord> = [
        (PitchClass::A, ChordQuality::Min),
        (PitchClass::F, ChordQuality::Maj),
        (PitchClass::C, ChordQuality::Maj),
        (PitchClass::G, ChordQuality::Maj),
    ]
    .iter()
    .enumerate()
    .map(|(i, &(root, quality))| TimedChord {
        chord: Chord::new(root, quality),
        start_beat: (i * 4) as u32,
        duration_beats: 4,
    })
    .collect();
    let scale = Some(Scale::new(PitchClass::A, Mode::Minor));
    let params = melody_params(0.5, 0.7, ContourPreference::Auto);
    for seed in 0..200 {
        let source = MotifSource::Generated(MotifParams {
            seed,
            complexity: 0.7,
            motif_len: 0,
            leap_chance: 0.5,
        });
        let notes = derive_motif_melody_with_section(&chords, scale, &params, &source, seed, 480);
        assert!(!notes.is_empty(), "empty melody for seed {seed}");
        assert_single_climax(&notes, &format!("seed {seed}, A minor"));
    }
}

#[test]
fn climaxes_are_often_approached_by_leap() {
    // "Ideally approached by leap" is a soft rule — deepening aborts
    // whenever it would break the leap grammar — so assert a healthy
    // share rather than universality.
    let mut eligible = 0u32;
    let mut by_leap = 0u32;
    for seed in 0..400 {
        let notes = generate(seed, 0.3, 0.6, ContourPreference::Auto);
        let pitches: Vec<u8> = notes.iter().map(|n| n.note).collect();
        let n = pitches.len();
        if n < 3 {
            continue;
        }
        let max = *pitches.iter().max().unwrap();
        if max == *pitches.iter().min().unwrap() {
            continue;
        }
        let climax = pitches.iter().position(|&p| p == max).unwrap();
        if climax == 0 {
            continue;
        }
        eligible += 1;
        if (max as i16 - pitches[climax - 1] as i16) >= 3 {
            by_leap += 1;
        }
    }
    assert!(eligible > 200, "too few eligible phrases ({eligible})");
    let ratio = by_leap as f32 / eligible as f32;
    assert!(
        ratio >= 0.60,
        "only {by_leap}/{eligible} climaxes approached by leap"
    );
}

#[test]
fn climax_enforcement_stays_deterministic() {
    let a = generate(42, 0.5, 0.6, ContourPreference::Wave);
    let b = generate(42, 0.5, 0.6, ContourPreference::Wave);
    assert_eq!(a, b);
}
