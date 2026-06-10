// Cadence formula targeting (Open Music Theory v2: intro-to-harmony,
// strengthening-endings-with-v7): every phrase gets a goal cadence —
// weak (HC, sometimes IAC) for antecedents, strong (PAC, ~10%
// deceptive) for consequents — and the final two melody notes are
// forced to a two-note formula compatible with the underlying chord:
//
//   PAC        2→1, 7→1           closed: ends on the tonic
//   IAC        4→3, 2→3, 6→5, 4→5 softer close: ends on 3 or 5
//   HC         1→7, 3→2           open: asks the next phrase to resolve
//   Deceptive  7→6, 5→6           the V→vi surprise, melody-side: lands on 6
//
// Every formula approaches the final by a scale step, so a forced
// cadence composes with the leap grammar (`apply_leap_recovery`) and
// the single-climax pass (`enforce_single_climax`) instead of fighting
// them. The formulas also encode the tendency-tone resolutions 7→1,
// 4→3, 2→1; candidate scoring additionally prefers a formula whose
// approach tone resolves a tendency tone left hanging by the note
// before the cadence pair.
//
// This module is the shared table + candidate enumerator; the
// instrumental overlay lives in `motif_engine::cadence`, the vocal one
// in `vocal::melody::apply_line_cadence_formulas`.

use crate::chord::Chord;
use crate::rng::XorShift;
use crate::scale::Scale;

use super::bass::step_scale;

/// Share of consequent phrases that get a deceptive ending instead of
/// the authentic close.
const DECEPTIVE_CHANCE: f32 = 0.10;

/// Goal cadence for one phrase, ordered weak → strong-ish. Strength
/// order per OMT: HC < IAC < PAC; Deceptive is a subverted PAC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::derive) enum CadenceGoal {
    Pac,
    Iac,
    Hc,
    Deceptive,
}

impl CadenceGoal {
    /// The melodic formulas of this cadence as `(penult_degree,
    /// final_degree)` pairs, 1-based diatonic degrees. Order = priority.
    pub(in crate::derive) fn formulas(self) -> &'static [(u8, u8)] {
        match self {
            CadenceGoal::Pac => &[(2, 1), (7, 1)],
            CadenceGoal::Iac => &[(4, 3), (2, 3), (6, 5), (4, 5)],
            CadenceGoal::Hc => &[(1, 7), (3, 2)],
            CadenceGoal::Deceptive => &[(7, 6), (5, 6)],
        }
    }

    /// Fallback order when the primary goal has no formula compatible
    /// with the phrase-final chord. Every diatonic triad contains at
    /// least one final degree from {1, 2, 3, 5, 7}, so walking the
    /// chain always finds a compatible goal over diatonic harmony.
    pub(in crate::derive) fn chain(self) -> &'static [CadenceGoal] {
        match self {
            CadenceGoal::Pac => &[CadenceGoal::Pac, CadenceGoal::Iac, CadenceGoal::Hc],
            CadenceGoal::Iac => &[CadenceGoal::Iac, CadenceGoal::Pac, CadenceGoal::Hc],
            CadenceGoal::Hc => &[CadenceGoal::Hc, CadenceGoal::Iac, CadenceGoal::Pac],
            CadenceGoal::Deceptive => &[
                CadenceGoal::Deceptive,
                CadenceGoal::Pac,
                CadenceGoal::Iac,
                CadenceGoal::Hc,
            ],
        }
    }
}

/// Pick a goal cadence for one phrase. Antecedents end weak (HC with
/// an occasional IAC); consequents end strong (PAC) with a ~10%
/// deceptive swap so periodic structures don't close identically
/// every time.
pub(in crate::derive) fn plan_cadence_goal(is_consequent: bool, rng: &mut XorShift) -> CadenceGoal {
    let roll = rng.next_f32();
    if is_consequent {
        if roll < DECEPTIVE_CHANCE {
            CadenceGoal::Deceptive
        } else {
            CadenceGoal::Pac
        }
    } else if roll < 0.70 {
        CadenceGoal::Hc
    } else {
        CadenceGoal::Iac
    }
}

/// Pitch class of a 1-based scale degree.
pub(in crate::derive) fn scale_degree_pc(scale: &Scale, degree: u8) -> u8 {
    let intervals = scale.mode.intervals();
    let idx = (degree as usize - 1) % intervals.len();
    (scale.root.to_semitone() + intervals[idx]) % 12
}

/// 1-based scale degree of a MIDI note, or `None` when the note is
/// outside the scale.
pub(in crate::derive) fn scale_degree_of(scale: &Scale, midi: u8) -> Option<u8> {
    let pc = midi % 12;
    let root = scale.root.to_semitone();
    let offset = (pc + 12 - root) % 12;
    scale
        .mode
        .intervals()
        .iter()
        .position(|&iv| iv == offset)
        .map(|i| (i + 1) as u8)
}

/// Where a tendency tone wants to go: 7→1, 4→3, 2→1 (OMT). `None` for
/// degrees with no strong pull.
pub(in crate::derive) fn tendency_resolution(degree: u8) -> Option<u8> {
    match degree {
        7 => Some(1),
        4 => Some(3),
        2 => Some(1),
        _ => None,
    }
}

/// Is `degree` an acceptable *final* over `chord`? Chord tones always
/// fit. Degree 6 — the deceptive landing — additionally passes as an
/// added-sixth color provided it doesn't sit a semitone from any chord
/// tone (e.g. b6 against the 5th of a minor tonic).
pub(in crate::derive) fn final_degree_fits_chord(scale: &Scale, degree: u8, chord: Chord) -> bool {
    let pc = scale_degree_pc(scale, degree);
    let mut clash = false;
    for c in chord.pitch_classes() {
        let c = c.to_semitone();
        if c == pc {
            return true;
        }
        let d = (pc as i16 - c as i16).rem_euclid(12);
        if d == 1 || d == 11 {
            clash = true;
        }
    }
    degree == 6 && !clash
}

/// One realizable cadence ending: concrete MIDI pitches for the final
/// two notes plus the formula and fallback rank they came from.
#[derive(Debug, Clone, Copy)]
pub(in crate::derive) struct FormulaCandidate {
    pub(in crate::derive) penult: u8,
    pub(in crate::derive) fin: u8,
    pub(in crate::derive) penult_degree: u8,
    /// Index into `goal.chain()`: 0 = the planned goal itself.
    pub(in crate::derive) goal_rank: usize,
}

/// Enumerate every in-register realization of the goal's formulas
/// (walking the fallback chain) whose final degree is compatible with
/// `chord`. The penult is the scale step adjacent to the final on the
/// side the formula prescribes — by construction the cadence move is a
/// step (an augmented second at worst, in harmonic minor).
pub(in crate::derive) fn formula_candidates(
    goal: CadenceGoal,
    scale: &Scale,
    chord: Chord,
    register: (u8, u8),
) -> Vec<FormulaCandidate> {
    let (lo, hi) = register;
    let mut out = Vec::new();
    for (goal_rank, g) in goal.chain().iter().enumerate() {
        for &(p_deg, f_deg) in g.formulas() {
            if !final_degree_fits_chord(scale, f_deg, chord) {
                continue;
            }
            let f_pc = scale_degree_pc(scale, f_deg);
            let p_pc = scale_degree_pc(scale, p_deg);
            for fin in lo..=hi {
                if fin % 12 != f_pc {
                    continue;
                }
                // The approach tone is the adjacent scale step whose
                // pitch class matches the formula's penult degree.
                for dir in [1, -1] {
                    let penult = step_scale(scale, fin, dir);
                    if penult == fin || penult % 12 != p_pc || penult < lo || penult > hi {
                        continue;
                    }
                    out.push(FormulaCandidate {
                        penult,
                        fin,
                        penult_degree: p_deg,
                        goal_rank,
                    });
                }
            }
        }
    }
    out
}
