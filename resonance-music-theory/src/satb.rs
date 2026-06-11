//! SATB-style voice-leading pass for harmony rendering.
//!
//! Implements the chorale-derived rules from Open Music Theory's
//! "chords in SATB style" (research §2G): the bass is planned first
//! (chord root, or the slash bass), the top voice is planned *backwards
//! from the cadence* so tendency tones resolve correctly (leading tone
//! up a semitone, chordal seventh down a step, final note prefers the
//! tonic), and the inner voices fill in the nearest chord tones.
//!
//! Voicings are chosen by dynamic programming over the whole
//! progression (per-chord candidate voicings, Viterbi over transition
//! costs) rather than greedily per chord — an early voicing is never
//! allowed to trap a later chord into parallels. The scoring encodes:
//!
//! - parallel fifths/octaves between any voice pair are heavily
//!   penalised (effectively forbidden whenever an alternative exists);
//! - the leading tone and a chordal seventh are never doubled;
//! - a chordal seventh resolves down by step in the same voice;
//! - contrary motion is preferred against a rising 4→5 bass;
//! - voices move as little as possible otherwise.
//!
//! The pass is deterministic — same input, same voicings — and is
//! consumed by the Pad lane renderer (`derive_pad`), turning chord
//! lanes into voiced parts instead of parallel block stacks.

use crate::chord::{Chord, ChordQuality};
use crate::pitch::PitchClass;
use crate::scale::Scale;
use crate::voicing::{nearest_midi_above, nearest_midi_to};

/// Cost weights. Hard rules get penalties large enough that the search
/// only violates them when every candidate does (degenerate registers).
const W_MOVEMENT: i64 = 3;
const W_PLAN_DEVIATION: i64 = 4;
const W_SPACING: i64 = 8;
const PENALTY_PARALLEL: i64 = 100_000;
const PENALTY_DOUBLED_TENDENCY: i64 = 20_000;
const PENALTY_MISSING_CORE_TONE: i64 = 10_000;
const PENALTY_MISSING_FIFTH: i64 = 50;
const PENALTY_UNRESOLVED_SEVENTH: i64 = 5_000;
const PENALTY_UNRESOLVED_SOPRANO_LT: i64 = 5_000;
const PENALTY_UNRESOLVED_CADENTIAL_64: i64 = 5_000;
const PENALTY_UPPER_BELOW_BASS: i64 = 200;
const PENALTY_RISING_AGAINST_4_5_BASS: i64 = 40;

/// Cap on candidate voicings kept per chord for the DP. Candidates are
/// sorted by intrinsic cost first, so the cap only trims pathological
/// registers.
const MAX_STATES: usize = 64;

/// Voice the whole progression in SATB style. Returns one voicing per
/// chord; each voicing lists MIDI notes bass-first in *voice* order
/// (bass, then upper voices ascending). Four voices when `register`
/// spans at least 16 semitones, three otherwise (narrow pads stay
/// close-voiced with one note per chord tone, no doubling).
///
/// `scale` enables the key-dependent rules (leading-tone treatment,
/// tonic-preferring cadence, 4→5 bass detection); without it the pass
/// still voice-leads, forbids parallels, and resolves chordal sevenths.
pub fn satb_voicings(
    chords: &[Chord],
    scale: Option<Scale>,
    register: (u8, u8),
) -> Vec<Vec<u8>> {
    if chords.is_empty() {
        return Vec::new();
    }
    let lo = register.0.min(register.1);
    let hi = register.0.max(register.1);
    let n_voices: usize = if hi - lo >= 16 { 4 } else { 3 };

    let bass_line = plan_bass(chords, lo, hi);
    let soprano_plan = plan_soprano(chords, scale, lo, hi);

    // Per-chord candidate voicings with their intrinsic (state) costs.
    let states: Vec<Vec<Candidate>> = chords
        .iter()
        .enumerate()
        .map(|(i, &chord)| {
            candidates_for(chord, bass_line[i], soprano_plan[i], scale, (lo, hi), n_voices)
        })
        .collect();

    // Viterbi over transitions: cost[s] = best path cost ending in s.
    let mut cost: Vec<i64> = states[0].iter().map(|c| c.cost).collect();
    let mut back: Vec<Vec<usize>> = Vec::with_capacity(chords.len());
    back.push(Vec::new());
    for i in 1..chords.len() {
        let rising_4_5 = is_rising_4_to_5(scale, bass_line[i - 1], bass_line[i]);
        let mut next_cost = vec![i64::MAX; states[i].len()];
        let mut next_back = vec![0usize; states[i].len()];
        for (si, s) in states[i].iter().enumerate() {
            for (pi, p) in states[i - 1].iter().enumerate() {
                if cost[pi] == i64::MAX {
                    continue;
                }
                let t = transition_cost(p, s, chords[i - 1], chords[i], scale, rising_4_5);
                let total = cost[pi].saturating_add(t).saturating_add(s.cost);
                if total < next_cost[si] {
                    next_cost[si] = total;
                    next_back[si] = pi;
                }
            }
        }
        cost = next_cost;
        back.push(next_back);
    }

    // Backtrack the cheapest full path.
    let mut idx = cost
        .iter()
        .enumerate()
        .min_by_key(|(_, &c)| c)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let mut path = vec![0usize; chords.len()];
    for i in (0..chords.len()).rev() {
        path[i] = idx;
        if i > 0 {
            idx = back[i][idx];
        }
    }
    path.iter()
        .enumerate()
        .map(|(i, &s)| states[i][s].notes.clone())
        .collect()
}

/// The pitch class of the chordal seventh, when the quality has one.
/// Sixths (Maj6/Min6) and the added ninth are *not* sevenths — only
/// true sevenths carry the resolve-down-by-step obligation.
pub fn chordal_seventh(chord: Chord) -> Option<PitchClass> {
    use ChordQuality::*;
    let interval = match chord.quality {
        Maj7 | MinMaj7 => 11,
        Min7 | Dom7 | HalfDim7 => 10,
        Dim7 => 9,
        _ => return None,
    };
    Some(chord.root.transpose(interval))
}

/// One candidate voicing for a single chord: bass-first notes plus the
/// chord-intrinsic cost (doubling, coverage, spacing, plan deviation).
struct Candidate {
    notes: Vec<u8>,
    cost: i64,
}

/// The leading tone of the key: a semitone below the tonic. Defined
/// even in modes whose diatonic seventh degree is lowered — a borrowed
/// V chord's raised seventh still behaves as a leading tone.
fn leading_tone(scale: Scale) -> PitchClass {
    scale.root.transpose(11)
}

/// The chord's "third" — the first interval above the root (a real
/// third for tertian chords, the 2nd/4th for sus chords). Core tone
/// that must be present for the chord quality to read.
fn chord_third(chord: Chord) -> PitchClass {
    chord.root.transpose(chord.quality.intervals()[1] as i32)
}

/// The perfect fifth above the root, when the quality contains one.
/// The only tone allowed to drop out when voices run short.
fn chord_fifth(chord: Chord) -> Option<PitchClass> {
    chord
        .quality
        .intervals()
        .contains(&7)
        .then(|| chord.root.transpose(7))
}

/// Shift `midi` by octaves into `[lo, hi]`; clamps to the nearest edge
/// when the window is narrower than an octave and no instance fits.
fn fold_into(midi: u8, lo: u8, hi: u8) -> u8 {
    let mut m = midi as i32;
    while m < lo as i32 {
        m += 12;
    }
    while m > hi as i32 {
        m -= 12;
    }
    m.clamp(lo as i32, hi as i32).clamp(0, 127) as u8
}

/// Every instance of `pc` inside `[lo, hi]`, ascending.
fn instances(pc: PitchClass, lo: u8, hi: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(3);
    let mut n = nearest_midi_above(pc, lo);
    while n <= hi {
        out.push(n);
        let Some(up) = n.checked_add(12) else { break };
        n = up;
    }
    out
}

/// Is `prev → next` a 6/4 resolution over a stationary bass? The
/// previous chord is a triad voiced over its own fifth (the 6/4
/// position — most importantly the cadential 6/4, a tonic triad over
/// the dominant bass) and the next chord is rooted on that same bass
/// pitch with no slash of its own: I6/4 → V over a held bass 5.
fn is_cadential_64_resolution(prev: Chord, next: Chord) -> bool {
    let Some(bass) = prev.bass else { return false };
    let Some(fifth) = chord_fifth(prev) else {
        return false;
    };
    bass == fifth && next.root == bass && next.bass.unwrap_or(next.root) == bass
}

/// Did the bass rise from scale degree 4 to scale degree 5?
fn is_rising_4_to_5(scale: Option<Scale>, prev_bass: u8, bass: u8) -> bool {
    let Some(s) = scale else { return false };
    let deg4 = s.root.transpose(5).to_semitone();
    let deg5 = s.root.transpose(7).to_semitone();
    prev_bass % 12 == deg4 && bass % 12 == deg5 && bass > prev_bass
}

/// Bass first: the slash bass when present, otherwise the root, kept
/// inside the bottom octave of the register and moving as little as
/// possible from chord to chord.
fn plan_bass(chords: &[Chord], lo: u8, hi: u8) -> Vec<u8> {
    let zone_hi = (lo + 12).min(hi);
    let mut line = Vec::with_capacity(chords.len());
    let mut prev: Option<u8> = None;
    for chord in chords {
        let pc = chord.bass.unwrap_or(chord.root);
        let note = match prev {
            None => fold_into(nearest_midi_above(pc, lo), lo, zone_hi),
            Some(p) => fold_into(nearest_midi_to(pc, p), lo, zone_hi),
        };
        prev = Some(note);
        line.push(note);
    }
    line
}

/// Melody second, built backwards from the cadence: pick the final
/// soprano note first (tonic if the last chord contains it, else its
/// third, else its root), then walk backwards choosing chord tones
/// near the *following* note, refusing tendency tones that would not
/// resolve — a leading tone is only planned where the next note is a
/// semitone above it, a chordal seventh only where the next note is a
/// step below.
fn plan_soprano(chords: &[Chord], scale: Option<Scale>, lo: u8, hi: u8) -> Vec<u8> {
    let zone_lo = hi.saturating_sub(12).max(lo);
    let zone_hi = hi;
    let center = ((zone_lo as u32 + zone_hi as u32) / 2) as u8;
    let mut plan = vec![0u8; chords.len()];

    // Final note: tonic > third > root preference, nearest the zone
    // center so the cadence lands mid-register, not at an extreme.
    let last = chords[chords.len() - 1];
    let last_pcs: Vec<PitchClass> = last.pitch_classes().collect();
    let goal_pc = scale
        .map(|s| s.root)
        .filter(|tonic| last_pcs.contains(tonic))
        .unwrap_or_else(|| chord_third(last));
    plan[chords.len() - 1] = fold_into(nearest_midi_to(goal_pc, center), zone_lo, zone_hi);

    let lt = scale.map(leading_tone);
    for i in (0..chords.len().saturating_sub(1)).rev() {
        let next_note = plan[i + 1];
        let seventh = chordal_seventh(chords[i]);
        let mut best: Option<(i64, u8)> = None;
        for pc in chords[i].pitch_classes() {
            for cand in instances(pc, zone_lo, zone_hi) {
                let dist = (cand as i64 - next_note as i64).abs();
                let mut cost = dist;
                // Mild leap aversion: the soprano plan should be singable.
                if dist > 4 {
                    cost += 2;
                }
                if lt == Some(pc) && next_note as i64 != cand as i64 + 1 {
                    cost += 50; // leading tone that wouldn't resolve up
                }
                if seventh == Some(pc) {
                    let resolves_down = (1..=2).contains(&dist) && next_note < cand;
                    if !resolves_down {
                        cost += 50; // seventh that wouldn't resolve down
                    }
                }
                if best.is_none_or(|(c, _)| cost < c) {
                    best = Some((cost, cand));
                }
            }
        }
        plan[i] = best.map(|(_, n)| n).unwrap_or(next_note);
    }
    plan
}

/// Enumerate every strictly-ascending combination of `k` chord-tone
/// instances over the bass and score each one. Returns at most
/// `MAX_STATES` candidates, cheapest first, deterministic order.
fn candidates_for(
    chord: Chord,
    bass: u8,
    soprano_target: u8,
    scale: Option<Scale>,
    (lo, hi): (u8, u8),
    n_voices: usize,
) -> Vec<Candidate> {
    let n_upper = n_voices - 1;
    let pcs: Vec<PitchClass> = {
        let mut v: Vec<PitchClass> = chord.pitch_classes().collect();
        v.dedup();
        v
    };
    let lt = scale.map(leading_tone);
    let seventh = chordal_seventh(chord);
    let third = chord_third(chord);
    let fifth = chord_fifth(chord);

    // Pool of distinct chord-tone instances (excluding the bass note
    // itself — unisons with the bass would emit duplicate MIDI notes).
    let mut pool: Vec<u8> = Vec::new();
    for &pc in &pcs {
        for n in instances(pc, lo, hi) {
            if n != bass && !pool.contains(&n) {
                pool.push(n);
            }
        }
    }
    pool.sort_unstable();

    let mut out: Vec<Candidate> = Vec::new();
    let mut combo = vec![0usize; n_upper];
    enumerate_combinations(&pool, n_upper, 0, &mut combo, 0, &mut |upper| {
        let count_pc = |target: PitchClass| -> usize {
            let t = target.to_semitone();
            usize::from(bass % 12 == t) + upper.iter().filter(|&&n| n % 12 == t).count()
        };

        let mut cost: i64 = 0;

        // Doubling rules: never the leading tone, never a chordal 7th.
        if let Some(lt_pc) = lt {
            if count_pc(lt_pc) > 1 {
                cost += PENALTY_DOUBLED_TENDENCY;
            }
        }
        if let Some(sev) = seventh {
            if count_pc(sev) > 1 {
                cost += PENALTY_DOUBLED_TENDENCY;
            }
        }

        // Coverage: root and third always; the seventh when the chord
        // has one; the fifth may drop out (cheaply) when voices run
        // short.
        if count_pc(chord.root) == 0 {
            cost += PENALTY_MISSING_CORE_TONE;
        }
        if count_pc(third) == 0 {
            cost += PENALTY_MISSING_CORE_TONE;
        }
        if let Some(sev) = seventh {
            if count_pc(sev) == 0 {
                cost += PENALTY_MISSING_CORE_TONE;
            }
        }
        if let Some(f) = fifth {
            if count_pc(f) == 0 {
                cost += PENALTY_MISSING_FIFTH;
            }
        }

        // Spacing: adjacent upper voices within an octave, tenor at
        // most a twelfth above the bass; an upper voice dipping below
        // the bass is a last resort for degenerate registers.
        if upper[0] > bass {
            let gap = (upper[0] - bass) as i64;
            if gap > 19 {
                cost += (gap - 19) * (W_SPACING / 2);
            }
        } else {
            cost += PENALTY_UPPER_BELOW_BASS;
        }
        for w in upper.windows(2) {
            let gap = (w[1] - w[0]) as i64;
            if gap > 12 {
                cost += (gap - 12) * W_SPACING;
            }
        }

        // The planned soprano is a preference, not a straitjacket —
        // the DP may bend it to dodge parallels.
        let soprano = upper[n_upper - 1];
        cost += (soprano as i64 - soprano_target as i64).abs() * W_PLAN_DEVIATION;

        let notes: Vec<u8> = std::iter::once(bass).chain(upper.iter().copied()).collect();
        out.push(Candidate { notes, cost });
    });

    if out.is_empty() {
        // Degenerate register (fewer distinct instances than voices):
        // fall back to a close stack above the bass.
        let notes: Vec<u8> = std::iter::once(bass)
            .chain(
                pcs.iter()
                    .take(n_upper)
                    .map(|&pc| fold_into(nearest_midi_above(pc, bass.saturating_add(1)), lo, hi)),
            )
            .collect();
        out.push(Candidate { notes, cost: 0 });
    }

    out.sort_by(|a, b| a.cost.cmp(&b.cost).then_with(|| a.notes.cmp(&b.notes)));
    out.truncate(MAX_STATES);
    out
}

/// Visit every ascending `k`-combination of `pool` (which is sorted).
fn enumerate_combinations(
    pool: &[u8],
    k: usize,
    start: usize,
    combo: &mut [usize],
    depth: usize,
    visit: &mut impl FnMut(&[u8]),
) {
    if depth == k {
        let upper: Vec<u8> = combo.iter().map(|&i| pool[i]).collect();
        visit(&upper);
        return;
    }
    for i in start..pool.len() {
        combo[depth] = i;
        enumerate_combinations(pool, k, i + 1, combo, depth + 1, visit);
    }
}

/// True when the voice pair forms consecutive perfect fifths/octaves
/// (interval class preserved while both voices move).
fn is_parallel_perfect(pa: u8, pb: u8, na: u8, nb: u8) -> bool {
    if pa == na || pb == nb {
        return false; // oblique motion (or a repeated chord) is fine
    }
    let prev_ic = (pa as i32 - pb as i32).unsigned_abs() % 12;
    let next_ic = (na as i32 - nb as i32).unsigned_abs() % 12;
    prev_ic == next_ic && (prev_ic == 0 || prev_ic == 7)
}

/// Cost of moving from one voicing to the next: voice movement,
/// parallels, tendency-tone resolution, contrary motion vs a rising
/// 4→5 bass. Voice identity is positional (bass-first).
fn transition_cost(
    prev: &Candidate,
    next: &Candidate,
    prev_chord: Chord,
    next_chord: Chord,
    scale: Option<Scale>,
    rising_4_5_bass: bool,
) -> i64 {
    let pv = &prev.notes;
    let nv = &next.notes;
    let mut cost: i64 = 0;

    // Movement cost above the bass (the bass line is already planned).
    cost += pv[1..]
        .iter()
        .zip(nv[1..].iter())
        .map(|(&p, &n)| (p as i64 - n as i64).abs())
        .sum::<i64>()
        * W_MOVEMENT;

    // Parallel perfect fifths/octaves across every voice pair.
    for a in 0..pv.len().min(nv.len()) {
        for b in (a + 1)..pv.len().min(nv.len()) {
            if is_parallel_perfect(pv[a], pv[b], nv[a], nv[b]) {
                cost += PENALTY_PARALLEL;
            }
        }
    }

    // Tendency tones sounding in the previous chord must resolve in
    // the same voice: chordal sevenths step down (or hold as a common
    // tone), the soprano leading tone rises to the tonic.
    let prev_seventh = chordal_seventh(prev_chord);
    let next_pcs: Vec<PitchClass> = next_chord.pitch_classes().collect();
    let lt = scale.map(leading_tone);
    let tonic = scale.map(|s| s.root);
    for (v, (&p, &n)) in pv.iter().zip(nv.iter()).enumerate() {
        if let Some(sev) = prev_seventh {
            if p % 12 == sev.to_semitone() {
                let held_common_tone =
                    n == p && next_pcs.iter().any(|pc| pc.to_semitone() == p % 12);
                let steps_down = n < p && p - n <= 2;
                if !steps_down && !held_common_tone {
                    cost += PENALTY_UNRESOLVED_SEVENTH;
                }
            }
        }
        if let (Some(lt_pc), Some(tonic_pc)) = (lt, tonic) {
            let is_soprano = v == pv.len() - 1;
            if is_soprano
                && next_pcs.contains(&tonic_pc)
                && p % 12 == lt_pc.to_semitone()
                && n as i64 != p as i64 + 1
            {
                cost += PENALTY_UNRESOLVED_SOPRANO_LT;
            }
        }
    }

    // Cadential 6/4 resolution: over the stationary dominant bass the
    // 6th above the bass falls to the 5th and the 4th falls to the 3rd
    // *in the same voices* — i.e. every upper voice holding the 6/4
    // chord's root (the 4th over the bass) or its third (the 6th over
    // the bass) must step down. The bass itself holds (oblique motion),
    // which `plan_bass` already produces for a repeated pitch class.
    if is_cadential_64_resolution(prev_chord, next_chord) {
        let fourth_over_bass = prev_chord.root.to_semitone();
        let sixth_over_bass = chord_third(prev_chord).to_semitone();
        for (&p, &n) in pv[1..].iter().zip(nv[1..].iter()) {
            let pc = p % 12;
            if (pc == fourth_over_bass || pc == sixth_over_bass) && !(n < p && p - n <= 2) {
                cost += PENALTY_UNRESOLVED_CADENTIAL_64;
            }
        }
    }

    // Contrary motion against a rising 4→5 bass.
    if rising_4_5_bass {
        for (&p, &n) in pv[1..].iter().zip(nv[1..].iter()) {
            if n > p {
                cost += PENALTY_RISING_AGAINST_4_5_BASS;
            }
        }
    }

    cost
}
