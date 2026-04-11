//! Chord progression generation via a functional harmony walker.
//!
//! Given a scale we derive each scale degree's diatonic triad (or seventh
//! chord) directly from the mode's interval pattern, then sample a
//! sequence using a weighted tonic / subdominant / dominant transition
//! matrix. Progressions always start and end on the tonic function and
//! bias the penultimate chord toward the dominant so cadences resolve.

use crate::chord::{Chord, ChordQuality};
use crate::rng::XorShift;
use crate::scale::Scale;

/// Tonal function a scale degree occupies inside the T–S–D model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Function {
    Tonic = 0,
    Subdominant = 1,
    Dominant = 2,
}

impl Function {
    fn from_index(i: usize) -> Self {
        match i {
            0 => Function::Tonic,
            1 => Function::Subdominant,
            _ => Function::Dominant,
        }
    }
}

/// Functional role of a diatonic degree. Both major and minor modes map
/// the same degree numbers to the same buckets (I/iii/vi = T, ii/IV = S,
/// V/vii° = D), so this classification needs no scale argument.
pub fn degree_function(degree: u8) -> Function {
    match (degree.saturating_sub(1) % 7) + 1 {
        1 | 3 | 6 => Function::Tonic,
        2 | 4 => Function::Subdominant,
        5 | 7 => Function::Dominant,
        _ => Function::Tonic,
    }
}

/// Weighted transition probabilities, rows sum to 1.0.
/// Outer index is the current function, inner index is the next. Tuned
/// by ear to produce progressions that feel goal-directed without being
/// mechanical: tonics lean toward subdominant, subdominants push hard
/// toward dominant, dominants resolve almost always to tonic.
pub const TRANSITIONS: [[f32; 3]; 3] = [
    //                   T     S     D
    /* from Tonic       */ [0.15, 0.60, 0.25],
    /* from Subdominant */ [0.15, 0.15, 0.70],
    /* from Dominant    */ [0.80, 0.05, 0.15],
];

/// Parameters for [`walk_progression`]. `seed` is the sole source of
/// randomness so the same params always produce the same chords.
#[derive(Debug, Clone)]
pub struct ProgressionParams {
    pub scale: Scale,
    pub chord_count: u32,
    /// Build seventh chords instead of triads where the scale yields them.
    pub seventh_chords: bool,
    pub seed: u64,
}

/// Build the diatonic chord sitting on `degree` (1-indexed 1..=7) of the
/// scale. `seventh` extends the triad to a 7th chord when the scale's
/// 7th scale-tone forms a recognisable quality.
pub fn diatonic_chord(scale: Scale, degree: u8, seventh: bool) -> Chord {
    let ivs = scale.mode.intervals();
    let d = ((degree.saturating_sub(1)) % 7) as usize;
    let root_offset = ivs[d] as i32;

    // Scale tones are stacked in thirds above the root. Every time the
    // lookup wraps past index 6 we cross an octave, so add 12 for each
    // wrap — `offset_at` formalises that.
    let offset_at = |step: usize| -> i32 {
        let idx = (d + step) % 7;
        let octaves = ((d + step) / 7) as i32;
        ivs[idx] as i32 + 12 * octaves
    };

    let third_iv = (offset_at(2) - root_offset).rem_euclid(12);
    let fifth_iv = (offset_at(4) - root_offset).rem_euclid(12);
    let root_pc = scale.root.transpose(root_offset);

    let quality = if seventh {
        let seventh_iv = (offset_at(6) - root_offset).rem_euclid(12);
        seventh_quality(third_iv, fifth_iv, seventh_iv)
    } else {
        triad_quality(third_iv, fifth_iv)
    };

    Chord::new(root_pc, quality)
}

fn triad_quality(third: i32, fifth: i32) -> ChordQuality {
    match (third, fifth) {
        (4, 7) => ChordQuality::Maj,
        (3, 7) => ChordQuality::Min,
        (3, 6) => ChordQuality::Dim,
        (4, 8) => ChordQuality::Aug,
        // Exotic modes can yield chords that don't match a named quality;
        // fall back on the nearest triad by the 3rd alone.
        (4, _) => ChordQuality::Maj,
        _ => ChordQuality::Min,
    }
}

fn seventh_quality(third: i32, fifth: i32, seventh: i32) -> ChordQuality {
    match (third, fifth, seventh) {
        (4, 7, 11) => ChordQuality::Maj7,
        (3, 7, 10) => ChordQuality::Min7,
        (4, 7, 10) => ChordQuality::Dom7,
        (3, 7, 11) => ChordQuality::MinMaj7,
        (3, 6, 9) => ChordQuality::Dim7,
        (3, 6, 10) => ChordQuality::HalfDim7,
        _ => triad_quality(third, fifth),
    }
}

/// All seven diatonic triads of a scale, in degree order (I..vii°).
pub fn diatonic_triads(scale: Scale) -> [Chord; 7] {
    [
        diatonic_chord(scale, 1, false),
        diatonic_chord(scale, 2, false),
        diatonic_chord(scale, 3, false),
        diatonic_chord(scale, 4, false),
        diatonic_chord(scale, 5, false),
        diatonic_chord(scale, 6, false),
        diatonic_chord(scale, 7, false),
    ]
}

/// Degrees that belong to each function bucket. Same layout for major
/// and minor modes — the degree quality (major vs. minor chord) differs
/// between modes but the functional role does not.
fn degrees_for(func: Function) -> &'static [u8] {
    match func {
        Function::Tonic => &[1, 3, 6],
        Function::Subdominant => &[2, 4],
        Function::Dominant => &[5, 7],
    }
}

fn sample_transition(from: Function, rng: &mut XorShift) -> Function {
    let row = &TRANSITIONS[from as usize];
    let r = rng.next_f32();
    let mut acc = 0.0;
    for (i, &p) in row.iter().enumerate() {
        acc += p;
        if r < acc {
            return Function::from_index(i);
        }
    }
    Function::Tonic
}

fn chord_for_function(
    scale: Scale,
    func: Function,
    sevenths: bool,
    rng: &mut XorShift,
) -> Chord {
    let degrees = degrees_for(func);
    let degree = degrees[rng.next_range(degrees.len())];
    diatonic_chord(scale, degree, sevenths)
}

/// Produce a sequence of chords using the functional walker.
///
/// The first chord is always a tonic, the last chord is always a tonic,
/// and the second-to-last is biased strongly toward dominant so the
/// final resolution feels like an authentic cadence.
pub fn walk_progression(params: &ProgressionParams) -> Vec<Chord> {
    let count = params.chord_count.max(1) as usize;
    let mut rng = XorShift::new(params.seed);
    let mut result = Vec::with_capacity(count);

    // First chord: always degree I specifically (not a sampled
    // tonic-function chord). Starting on iii or vi confuses the ear
    // about what key we're in — conventional progressions anchor
    // the opening to the root.
    let mut current = Function::Tonic;
    result.push(diatonic_chord(params.scale, 1, params.seventh_chords));

    for i in 1..count {
        let is_last = i == count - 1;
        let is_penultimate = count >= 3 && i == count - 2;

        let next = if is_last {
            Function::Tonic
        } else if is_penultimate {
            // 70% dominant to set up the cadence; otherwise fall through
            // to the regular transition so we don't always sound canned.
            if rng.next_f32() < 0.7 {
                Function::Dominant
            } else {
                sample_transition(current, &mut rng)
            }
        } else {
            sample_transition(current, &mut rng)
        };

        // For the final tonic, force degree 1 specifically — iii/vi don't
        // close a progression as cleanly as the root does.
        let chord = if is_last {
            diatonic_chord(params.scale, 1, params.seventh_chords)
        } else {
            chord_for_function(params.scale, next, params.seventh_chords, &mut rng)
        };

        result.push(chord);
        current = next;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::PitchClass;
    use crate::scale::Mode;

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
        assert_eq!(diatonic_chord(scale, 1, true), Chord::new(PitchClass::C, ChordQuality::Maj7));
        assert_eq!(diatonic_chord(scale, 2, true), Chord::new(PitchClass::D, ChordQuality::Min7));
        assert_eq!(diatonic_chord(scale, 5, true), Chord::new(PitchClass::G, ChordQuality::Dom7));
        assert_eq!(diatonic_chord(scale, 7, true), Chord::new(PitchClass::B, ChordQuality::HalfDim7));
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
            assert_eq!(chords.last().unwrap().root, PitchClass::C, "seed {seed} last chord");
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
        let p = ProgressionParams { scale, chord_count: 1, seventh_chords: false, seed: 0 };
        let chords = walk_progression(&p);
        assert_eq!(chords.len(), 1);
        assert_eq!(chords[0].root, PitchClass::C);
    }
}
