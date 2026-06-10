// Single-climax enforcement (Open Music Theory: well-formed melodic
// lines): every phrase gets exactly one highest note, placed in the
// phrase's second half but never on the final note, ideally approached
// by leap. Duplicate peaks (e.g. the wave contour's two crests) and
// first-half super-peaks are demoted by scale steps.
//
// The pass is demote-only — it never raises a pitch — so the phrase
// maximum is non-increasing across repeated applications. That is what
// lets `motif_engine::melody` alternate it with `apply_leap_recovery`
// to a fixpoint: the leap grammar's repairs step from a note at least
// a third below the maximum (or step downward), so they can never lift
// a pitch back up to the phrase maximum, and the alternation settles.
//
// Shared by the instrumental motif engine (per realized phrase) and
// the vocal generator (per lyric line). Vocal lines additionally rely
// on demotion never widening an adjacent interval beyond the SVS
// `MAX_INTERVAL` cap: a demoted note lands just below the window
// maximum, and every neighbor of a former peak sits within the cap of
// that peak, hence within the cap of the demoted value too. The one
// move that could widen an interval — the final-note octave drop —
// is guarded explicitly.

use crate::scale::Scale;

use super::bass::step_scale;
use super::motif_bass::chord_tones_in_register;
use super::{GeneratedNote, TimedChord};

/// Smallest melodic move that counts as a leap, in semitones.
const LEAP_MIN: i16 = 3;

/// Largest melodic move that counts as a step, in semitones.
const STEP_MAX: i16 = 2;

/// Widest interval the vocal SVS pipeline renders cleanly between
/// adjacent notes (`vocal::style::MAX_INTERVAL`). Demotion preserves
/// it structurally; the octave-drop shortcut checks it explicitly.
const MAX_ADJACENT: i16 = 9;

/// One scale step down from `note`; a semitone without a scale.
fn step_down(note: u8, scale: Option<Scale>) -> u8 {
    match scale {
        Some(s) => step_scale(&s, note, -1),
        None => note.saturating_sub(1),
    }
}

/// Harmonic context for callers that keep strong-beat notes on chord
/// tones (`align_to_harmony` in the motif engine). When provided,
/// demotion of a strong-beat note steps down to the nearest chord
/// tone below the climax instead of a scale tone, and the
/// leap-approach deepening leaves strong-beat predecessors alone.
/// The vocal generator passes `None` — its chord-tone anchoring is
/// probabilistic, not a contract.
///
/// Climax enforcement runs *before* the embellishment pass, so the
/// strict pre-decoration form of the strong-beat contract (strong
/// beats are chord tones, full stop) holds here. The embellishment
/// pass later relaxes it to "strong-beat dissonances resolve down by
/// step", validating its candidates against the climax rule so the
/// single climax survives decoration.
pub(in crate::derive) struct ClimaxHarmony<'a> {
    pub(in crate::derive) chords: &'a [TimedChord],
    pub(in crate::derive) tpb: u64,
    pub(in crate::derive) register: (u8, u8),
}

impl ClimaxHarmony<'_> {
    /// Is the note at `tick` on a strong beat of its chord? Mirrors
    /// `align_to_harmony`: strong = a multiple of 2 beats from the
    /// chord start.
    fn is_strong_beat(&self, tick: u64) -> bool {
        self.chord_at(tick).is_some_and(|tc| {
            let chord_start = tc.start_beat as u64 * self.tpb;
            self.tpb > 0 && (tick - chord_start).is_multiple_of(2 * self.tpb)
        })
    }

    fn chord_at(&self, tick: u64) -> Option<&TimedChord> {
        self.chords
            .iter()
            .rfind(|tc| tc.start_beat as u64 * self.tpb <= tick)
            .or_else(|| self.chords.first())
    }

    /// Highest chord tone strictly below `below` (and at or above the
    /// register floor) for the chord at `tick`.
    fn chord_tone_below(&self, tick: u64, below: u8) -> Option<u8> {
        let tc = self.chord_at(tick)?;
        chord_tones_in_register(tc.chord, self.register)
            .into_iter()
            .rfind(|&t| t < below && t >= self.register.0)
    }
}

/// Pitch span of the monotonic non-ascending run that would end at
/// `end` if it held `end_value` (repeats extend a run, matching the
/// leap grammar's run segmentation). 0 when the move into `end`
/// ascends. Used to keep the leap-approach deepening from leaving a
/// run that outlines a tritone or a seventh.
fn run_span_ending_at(notes: &[GeneratedNote], end: usize, end_value: u8) -> i16 {
    let pitch_at = |i: usize| -> i16 {
        if i == end {
            end_value as i16
        } else {
            notes[i].note as i16
        }
    };
    let mut j = end;
    while j > 0 && pitch_at(j) <= pitch_at(j - 1) {
        j -= 1;
    }
    end_value as i16 - pitch_at(j)
}

/// Enforce the single-climax rule on one phrase (instrumental) or one
/// lyric line (vocal): exactly one highest note, in the second half,
/// never the final note, ideally approached by leap. Only demotes —
/// the climax pitch is the second-half maximum and everything else at
/// or above it is stepped down below it. Returns whether any note
/// changed.
///
/// `leap_approach` opts into the "ideally approached by leap"
/// deepening (lowering the climax's predecessor until the approach is
/// a third). The motif engine enables it; the vocal generator leaves
/// it off because its style contracts (chant's narrow speaking band,
/// hymnal's stepwise motion) are worth more than a forced leap.
///
/// Skips (returning `false`) phrases that are too short to host a
/// non-final second-half climax (< 3 notes), completely flat lines
/// (chant-like recitation has no contour to discipline), and the
/// degenerate case where the window maximum sits on the register
/// floor so nothing could be demoted below it.
pub(in crate::derive) fn enforce_single_climax(
    notes: &mut [GeneratedNote],
    scale: Option<Scale>,
    register: (u8, u8),
    harmony: Option<&ClimaxHarmony<'_>>,
    leap_approach: bool,
) -> bool {
    let n = notes.len();
    if n < 3 {
        return false;
    }
    let lo = register.0;
    let phrase_max = notes.iter().map(|x| x.note).max().unwrap_or(0);
    let phrase_min = notes.iter().map(|x| x.note).min().unwrap_or(0);
    if phrase_max == phrase_min {
        return false;
    }

    // The climax window: second half of the phrase, final note excluded.
    let w_start = n / 2;
    let w_end = n - 1;
    let wmax = notes[w_start..w_end]
        .iter()
        .map(|x| x.note)
        .max()
        .unwrap_or(0);
    if wmax <= lo {
        return false;
    }

    // Choose the climax among the window notes at the window maximum:
    // prefer one already approached by leap, tie-break toward the
    // later position (longer build-up).
    let mut best: Option<(bool, usize)> = None;
    for i in w_start..w_end {
        if notes[i].note != wmax {
            continue;
        }
        let approached_by_leap = (wmax as i16 - notes[i - 1].note as i16).abs() >= LEAP_MIN;
        let key = (approached_by_leap, i);
        if best.is_none_or(|b| key > b) {
            best = Some(key);
        }
    }
    let Some((_, climax)) = best else {
        return false;
    };

    // Demote every other note at or above the climax pitch.
    let mut changed = false;
    for i in 0..n {
        if i == climax || notes[i].note < wmax {
            continue;
        }
        let original = notes[i].note;

        // The final note is usually a cadence landing (chord root /
        // formula degree); try an octave drop first so its pitch class
        // survives the demotion.
        if i == n - 1 {
            let dropped = original as i16 - 12;
            let approach = dropped - notes[i - 1].note as i16;
            if dropped >= lo as i16 && dropped < wmax as i16 && approach.abs() <= MAX_ADJACENT {
                notes[i].note = dropped as u8;
                changed = true;
                continue;
            }
        }

        // Strong-beat notes must stay chord tones (the motif engine's
        // `align_to_harmony` contract): demote straight to the highest
        // chord tone below the climax instead of walking scale steps.
        if let Some(h) = harmony {
            if h.is_strong_beat(notes[i].start_tick) {
                if let Some(tone) = h.chord_tone_below(notes[i].start_tick, wmax) {
                    if tone != original {
                        notes[i].note = tone;
                        changed = true;
                    }
                    continue;
                }
                // No chord tone below the climax: fall through to the
                // scale-step demotion — a unique climax outranks the
                // strong-beat chord-tone preference here.
            }
        }

        let mut p = original;
        for _ in 0..24 {
            if p < wmax || p <= lo {
                break;
            }
            let next = step_down(p, scale);
            if next >= p {
                break; // pinned by a scale/register edge
            }
            p = next;
        }
        let p = p.max(lo);
        if p < wmax && p != original {
            notes[i].note = p;
            changed = true;
        }
    }

    // "Ideally approached by leap": when the approach into the climax
    // is a step, deepen the preceding note by scale steps until the
    // approach reaches a minor 3rd — but only while the move into that
    // note stays a step (and, where its own predecessor leapt a 4th or
    // more, stays the opposite-direction recovery step the leap
    // grammar demands), and while no monotonic run is left outlining a
    // tritone or a seventh. Best-effort: any guard failing simply
    // leaves the step approach in place.
    let pred = climax - 1;
    let pred_is_strong = harmony.is_some_and(|h| h.is_strong_beat(notes[pred].start_tick));
    for _ in 0..2 {
        if !leap_approach || pred_is_strong {
            // Deepening disabled, or it would pull a strong beat off
            // its chord tone.
            break;
        }
        let approach = wmax as i16 - notes[pred].note as i16;
        if approach >= LEAP_MIN {
            break;
        }
        let cand = step_down(notes[pred].note, scale);
        if cand >= notes[pred].note || cand < lo {
            break;
        }
        if pred > 0 {
            let into_pred = cand as i16 - notes[pred - 1].note as i16;
            if into_pred.abs() > STEP_MAX {
                break;
            }
            if pred > 1 {
                let into_prev = notes[pred - 1].note as i16 - notes[pred - 2].note as i16;
                if into_prev.abs() >= 5 && into_pred.signum() != -into_prev.signum() {
                    break;
                }
            }
            if matches!(run_span_ending_at(notes, pred, cand).abs(), 6 | 10 | 11) {
                break;
            }
        }
        notes[pred].note = cand;
        changed = true;
    }

    changed
}
