//! Huron's descending tendency (Open Music Theory: melodic statistics):
//! generated motifs take descending steps slightly more often than
//! ascending ones. Exercised through [`motif_intervals`], which returns
//! exactly the interval contour `build_motif` produced.

use resonance_music_theory::{
    motif_intervals, Chord, ChordQuality, Mode, MotifParams, MotifSource, PitchClass, Scale,
};

fn params(seed: u64) -> MotifParams {
    MotifParams {
        seed,
        complexity: 0.5,
        motif_len: 0,
        // Low leap chance so most moves are steps, where the bias applies.
        leap_chance: 0.1,
    }
}

/// Count ascending vs descending *steps* (moves of one or two semitones)
/// across a large seed sweep. A leap (≥ 3 semitones) carries no
/// descending bias and is excluded.
fn count_steps(scale: Option<Scale>, chord: Chord) -> (usize, usize) {
    let mut up = 0usize;
    let mut down = 0usize;
    for seed in 0..3000u64 {
        let source = MotifSource::Generated(params(seed));
        let intervals = motif_intervals(&source, chord, scale);
        for pair in intervals.windows(2) {
            let delta = i16::from(pair[1]) - i16::from(pair[0]);
            match delta {
                1 | 2 => up += 1,
                -1 | -2 => down += 1,
                _ => {}
            }
        }
    }
    (up, down)
}

#[test]
fn generated_steps_descend_more_often_than_they_ascend() {
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let chord = Chord::new(PitchClass::C, ChordQuality::Maj);
    let (up, down) = count_steps(scale, chord);

    assert!(up + down > 1000, "too few steps sampled: up {up}, down {down}");
    assert!(
        down > up,
        "expected descending steps to outnumber ascending: up {up}, down {down}"
    );
    // The bias is soft, not overwhelming — descending should lead by a
    // clear but modest margin, not dominate.
    let ratio = down as f32 / up as f32;
    assert!(
        ratio > 1.05 && ratio < 1.8,
        "descending/ascending ratio {ratio:.3} outside the expected soft-bias band (up {up}, down {down})"
    );
}

#[test]
fn descending_bias_keeps_generation_deterministic() {
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let chord = Chord::new(PitchClass::C, ChordQuality::Maj);
    let source = MotifSource::Generated(params(7));
    let a = motif_intervals(&source, chord, scale);
    let b = motif_intervals(&source, chord, scale);
    assert_eq!(a, b);
}
