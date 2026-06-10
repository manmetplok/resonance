// Embellishing-tone decoration pass (Open Music Theory v2,
// embellishing tones). Runs *last* in the per-phrase pipeline — after
// the leap-grammar/climax fixpoint and the goal-cadence overlay — and
// re-classifies the surface from the OMT table instead of leaving the
// blanket chord/scale snap of `align_to_harmony` as the final word:
//
//   | Tone          | Approach | Leave               | Beat   |
//   |---------------|----------|---------------------|--------|
//   | Passing       | step     | step, same dir      | weak   |
//   | Neighbor      | step     | step, opposite      | weak   |
//   | Appoggiatura  | leap     | step down           | strong |
//   | Suspension    | held     | step down           | strong |
//   | Escape        | step     | leap, opposite      | weak   |
//   | Anticipation  | —        | becomes chord tone  | weak   |
//
// Probabilities are style-weighted (`EmbellishmentStyle`): folk leans
// on passing/neighbor tones, pop ballad on suspensions/appoggiaturas,
// jazz on anticipations/escape tones. Density scales with the lane's
// complexity knob.
//
// Like every pass before it, decoration rewrites pitches only — no
// notes are inserted or removed. Every candidate is validated against
// the whole phrase (leap grammar, single climax, dissonance
// discipline, strong-beat contract, register) and dropped when any
// invariant would break, so the pass composes with the upstream
// passes instead of needing another fixpoint round. The final two
// notes are never touched: they carry the cadence formula (or the
// consequent's root snap).
//
// This pass is what evolves the strong-beat contract from "strong
// beats are chord tones" to "strong-beat dissonances resolve by step"
// — appoggiaturas and suspensions deliberately place dissonance on
// the strong beat (the constructions here always resolve them *down*
// by step to a chord tone, the canonical shape; the contract itself
// only demands the step). The dissonance discipline
// (never leap both into and out of a dissonance) holds throughout:
// appoggiaturas leap in but step out, suspensions are held into,
// escape tones step in before leaping out, anticipations are left by
// repetition.

use crate::rng::XorShift;
use crate::scale::Scale;

use super::super::bass::step_scale;
use super::super::melody::EmbellishmentStyle;
use super::super::GeneratedNote;
use super::harmony::{
    climax_ok, dissonance_treatment_ok, leap_grammar_ok, strong_beats_ok, HarmonyGrid,
};

/// Smallest melodic move that counts as a leap, in semitones.
const LEAP_MIN: i16 = 3;
/// Largest melodic move that counts as a step, in semitones.
const STEP_MAX: i16 = 2;
/// Widest legal approach leap (a perfect 5th), matching the motif
/// builder's line rules. A 6-semitone (tritone) move is never legal.
const LEAP_MAX: i16 = 7;

fn is_step(mv: i16) -> bool {
    (1..=STEP_MAX).contains(&mv.abs())
}

fn is_legal_leap(mv: i16) -> bool {
    let a = mv.abs();
    (LEAP_MIN..=LEAP_MAX).contains(&a) && a != 6
}

/// Per-opportunity application probabilities for the embellishing-tone
/// table, before density scaling.
struct Weights {
    passing: f32,
    neighbor: f32,
    appoggiatura: f32,
    suspension: f32,
    escape: f32,
    anticipation: f32,
}

/// Style table (melody-generation-research.md §2D): folk decorates
/// with passing/neighbor motion only; pop ballad favors strong-beat
/// suspensions and appoggiaturas; jazz favors anticipations and
/// escape tones. Folk's strong-beat weights are exactly zero so a folk
/// surface keeps every strong beat consonant.
fn style_weights(style: EmbellishmentStyle) -> Weights {
    match style {
        EmbellishmentStyle::Folk => Weights {
            passing: 0.50,
            neighbor: 0.35,
            appoggiatura: 0.0,
            suspension: 0.0,
            escape: 0.0,
            anticipation: 0.0,
        },
        EmbellishmentStyle::PopBallad => Weights {
            passing: 0.18,
            neighbor: 0.10,
            appoggiatura: 0.32,
            suspension: 0.38,
            escape: 0.0,
            anticipation: 0.08,
        },
        EmbellishmentStyle::Jazz => Weights {
            passing: 0.15,
            neighbor: 0.08,
            appoggiatura: 0.08,
            suspension: 0.08,
            escape: 0.32,
            anticipation: 0.38,
        },
        // Resolved before this table is consulted.
        EmbellishmentStyle::Auto => style_weights(EmbellishmentStyle::Folk),
    }
}

/// Resolve `Auto` to a concrete style from the section's motif seed so
/// every lane in a section decorates with the same flavor. Splitmix-
/// style scramble first: nearby seeds must not all land on the same
/// style.
pub(super) fn resolve_embellishment_style(
    style: EmbellishmentStyle,
    seed: u64,
) -> EmbellishmentStyle {
    if style != EmbellishmentStyle::Auto {
        return style;
    }
    let mut s = seed.wrapping_add(0xD6E8_FEB8_6659_FD93);
    s = (s ^ (s >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    s = (s ^ (s >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    s ^= s >> 31;
    match s % 3 {
        0 => EmbellishmentStyle::Folk,
        1 => EmbellishmentStyle::PopBallad,
        _ => EmbellishmentStyle::Jazz,
    }
}

/// One candidate decoration: rewrite `notes[index]` to `pitch`, plus an
/// optional second rewrite (the suspension's delayed resolution).
struct Candidate {
    weight: f32,
    index: usize,
    pitch: u8,
    also: Option<(usize, u8)>,
}

/// Decorate a realized phrase with embellishing tones. `style` must be
/// concrete (resolve `Auto` with [`resolve_embellishment_style`]);
/// `complexity` scales the overall decoration density.
pub(super) fn apply_embellishments(
    notes: &mut [GeneratedNote],
    grid: &HarmonyGrid<'_>,
    scale: &Scale,
    register: (u8, u8),
    style: EmbellishmentStyle,
    complexity: f32,
    rng: &mut XorShift,
) {
    let n = notes.len();
    // The final two notes carry the cadence formula / root snap.
    if n < 4 {
        return;
    }
    let weights = style_weights(style);
    let density = 0.35 + 0.65 * complexity.clamp(0.0, 1.0);

    for i in 1..n - 2 {
        let candidates = collect_candidates(notes, i, grid, scale, register, &weights, n);

        // One roll per note: walk the cumulative density-scaled
        // weights; a roll past the total leaves the note undecorated.
        let roll = rng.next_f32();
        let mut acc = 0.0f32;
        let mut chosen: Option<&Candidate> = None;
        for cand in &candidates {
            acc += cand.weight * density;
            if roll < acc {
                chosen = Some(cand);
                break;
            }
        }
        let Some(cand) = chosen else {
            continue;
        };

        // Validate the whole modified phrase; drop the decoration when
        // any invariant would break.
        let mut pitches: Vec<u8> = notes.iter().map(|x| x.note).collect();
        pitches[cand.index] = cand.pitch;
        if let Some((j, p)) = cand.also {
            pitches[j] = p;
        }
        if leap_grammar_ok(&pitches)
            && climax_ok(&pitches)
            && dissonance_treatment_ok(&pitches, notes, grid)
            && strong_beats_ok(&pitches, notes, grid)
        {
            notes[cand.index].note = cand.pitch;
            if let Some((j, p)) = cand.also {
                notes[j].note = p;
            }
        }
    }
}

/// Gather every embellishment construction applicable at note `i`,
/// with its style weight. Beat strength routes the vocabulary: strong
/// beats host appoggiaturas and suspensions, weak beats host passing,
/// neighbor, escape, and anticipation tones.
fn collect_candidates(
    notes: &[GeneratedNote],
    i: usize,
    grid: &HarmonyGrid<'_>,
    scale: &Scale,
    register: (u8, u8),
    weights: &Weights,
    n: usize,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    let tick = notes[i].start_tick;
    let prev = notes[i - 1].note;
    let next = notes[i + 1].note;
    let next_tick = notes[i + 1].start_tick;
    let in_register = |p: u8| p >= register.0 && p <= register.1;
    let strong = grid.is_strong_beat(tick);

    if strong {
        // Suspension: hold the previous pitch over the strong beat
        // (the preparation), sounding a dissonance, then resolve down
        // by step to a chord tone on the *next* note. Two-note
        // rewrite, so the resolution must also stay clear of the
        // protected cadence tail.
        if weights.suspension > 0.0 && i + 1 < n - 2 && !grid.is_chord_tone(tick, prev) {
            let resolution = step_scale(scale, prev, -1);
            if resolution < prev
                && is_step(prev as i16 - resolution as i16)
                && in_register(resolution)
                && grid.is_chord_tone(next_tick, resolution)
            {
                out.push(Candidate {
                    weight: weights.suspension,
                    index: i,
                    pitch: prev,
                    also: Some((i + 1, resolution)),
                });
            }
        }

        // Appoggiatura: leap into the upper scale neighbor of the
        // following chord tone, resolving down by step. Leap in, step
        // out — the dissonance discipline's allowed shape.
        if weights.appoggiatura > 0.0 && grid.is_chord_tone(next_tick, next) {
            let cand = step_scale(scale, next, 1);
            let approach = cand as i16 - prev as i16;
            if cand > next
                && is_step(cand as i16 - next as i16)
                && in_register(cand)
                && !grid.is_chord_tone(tick, cand)
                && is_legal_leap(approach)
            {
                out.push(Candidate {
                    weight: weights.appoggiatura,
                    index: i,
                    pitch: cand,
                    also: None,
                });
            }
        }
        return out;
    }

    // Passing tone: fill a third between the surrounding notes with
    // the scale tone between them (step in, step out, same direction).
    if weights.passing > 0.0 {
        let span = next as i16 - prev as i16;
        if (3..=4).contains(&span.abs()) {
            let dir = span.signum() as i32;
            let mid = step_scale(scale, prev, dir);
            let step_in = mid as i16 - prev as i16;
            let step_out = next as i16 - mid as i16;
            if mid != notes[i].note
                && in_register(mid)
                && is_step(step_in)
                && is_step(step_out)
                && step_in.signum() == span.signum()
                && step_out.signum() == span.signum()
            {
                out.push(Candidate {
                    weight: weights.passing,
                    index: i,
                    pitch: mid,
                    also: None,
                });
            }
        }
    }

    // Neighbor tone: between two statements of the same pitch, step
    // away and back (upper and lower neighbor weighted equally).
    if weights.neighbor > 0.0 && prev == next {
        for dir in [1, -1] {
            let cand = step_scale(scale, prev, dir);
            if cand != notes[i].note && cand != prev && in_register(cand)
                && is_step(cand as i16 - prev as i16)
            {
                out.push(Candidate {
                    weight: weights.neighbor / 2.0,
                    index: i,
                    pitch: cand,
                    also: None,
                });
            }
        }
    }

    // Escape tone: step away from the line's direction, then leap back
    // in the opposite direction (step in, leap out).
    if weights.escape > 0.0 && prev != next {
        let line_dir = (next as i16 - prev as i16).signum() as i32;
        let cand = step_scale(scale, prev, -line_dir);
        let step_in = cand as i16 - prev as i16;
        let leave = next as i16 - cand as i16;
        if cand != notes[i].note
            && in_register(cand)
            && is_step(step_in)
            && is_legal_leap(leave)
            && leave.signum() == -step_in.signum()
        {
            out.push(Candidate {
                weight: weights.escape,
                index: i,
                pitch: cand,
                also: None,
            });
        }
    }

    // Anticipation: the last note before a chord change sounds the
    // next chord's tone early (left by repetition).
    if weights.anticipation > 0.0 {
        let crosses_boundary = match (grid.chord_at(tick), grid.chord_at(next_tick)) {
            (Some(a), Some(b)) => a.start_beat != b.start_beat,
            _ => false,
        };
        if crosses_boundary && grid.is_chord_tone(next_tick, next) && next != notes[i].note {
            let approach = next as i16 - prev as i16;
            if in_register(next) && (approach == 0 || is_step(approach) || is_legal_leap(approach))
            {
                out.push(Candidate {
                    weight: weights.anticipation,
                    index: i,
                    pitch: next,
                    also: None,
                });
            }
        }
    }

    out
}
