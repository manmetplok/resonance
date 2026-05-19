//! Chord voicings and voice leading.
//!
//! `close_voicing` spells a chord as a stack of MIDI notes within one
//! octave above a given floor. `voice_lead` is the core pad primitive:
//! given where the voices are now and which pitch classes the next
//! chord needs, it returns a new voicing that minimises total semitone
//! movement — the "smooth" feel of a well-written pad.

use crate::chord::Chord;
use crate::pitch::PitchClass;

/// Smallest MIDI note `>= floor` whose pitch class matches `pc`. The
/// result is always in `[floor, floor + 11]` and is clamped to a
/// legal MIDI key (≤ 127) so callers that feed the result straight
/// into `GeneratedNote::note` (e.g. `derive_bass`, `motif_bass`,
/// `bass`) can never emit an illegal note.
pub fn nearest_midi_above(pc: PitchClass, floor: u8) -> u8 {
    let floor_pc = floor % 12;
    let pc_val = pc.to_semitone();
    let diff = (pc_val + 12 - floor_pc) % 12;
    floor.saturating_add(diff).min(127)
}

/// The MIDI note closest to `target` (in either direction) whose pitch
/// class is `pc`. Ties prefer the upper note.
pub fn nearest_midi_to(pc: PitchClass, target: u8) -> u8 {
    let target_pc = (target % 12) as i32;
    let pc_val = pc.to_semitone() as i32;
    // Distance forward around the circle of semitones.
    let forward = (pc_val - target_pc).rem_euclid(12);
    // Shift in the range [-6, +6]: prefer the shorter direction.
    let shift: i32 = if forward <= 6 { forward } else { forward - 12 };
    let raw = target as i32 + shift;
    raw.clamp(0, 127) as u8
}

/// Spell `chord` as a close voicing sitting at or just above `floor`.
/// Notes are returned in ascending order. Root is always the lowest
/// voice when `floor` lands on (or below) the root's pitch class.
pub fn close_voicing(chord: Chord, floor: u8) -> Vec<u8> {
    let mut notes: Vec<u8> = chord
        .pitch_classes()
        .iter()
        .map(|&pc| nearest_midi_above(pc, floor))
        .collect();
    notes.sort_unstable();
    notes
}

/// Shift `midi` by whole octaves until it falls inside `[low, high]`.
/// Assumes the register is at least one octave wide; if the register
/// is narrower and no octave fits, the nearest edge is returned.
fn fit_in_register(midi: u8, low: u8, high: u8) -> u8 {
    let mut m = midi as i32;
    let lo = low as i32;
    let hi = high as i32;
    while m < lo {
        m += 12;
    }
    while m > hi {
        m -= 12;
    }
    m.clamp(0, 127) as u8
}

/// Voice-lead from `prev` to the chord described by `next_pcs`.
///
/// Returns exactly `prev.len()` MIDI notes. When the next chord has
/// fewer pitch classes than voices (e.g. triad → 4-voice pad), one
/// pitch class gets doubled; when it has more, one is dropped. The
/// choice of assignment minimises summed absolute semitone movement.
///
/// The output is always sorted ascending so the bass voice sits at
/// index 0.
pub fn voice_lead(prev: &[u8], next_pcs: &[PitchClass], register: (u8, u8)) -> Vec<u8> {
    if prev.is_empty() || next_pcs.is_empty() {
        return Vec::new();
    }
    let (lo, hi) = register;
    let n_voices = prev.len();
    let n_pcs = next_pcs.len();

    // For each voice, compute a candidate MIDI note for each pitch
    // class — the closest instance of that pitch class to where the
    // voice sits now, clamped into the register window.
    let candidates: Vec<Vec<u8>> = prev
        .iter()
        .map(|&p| {
            next_pcs
                .iter()
                .map(|&pc| fit_in_register(nearest_midi_to(pc, p), lo, hi))
                .collect()
        })
        .collect();

    // Search space: one pitch-class index per voice. For the small voice
    // counts we actually produce (<= 6) this is never more than 6^6 =
    // 46_656 options — a microsecond of brute force.
    let total: u64 = (n_pcs as u64)
        .checked_pow(n_voices as u32)
        .unwrap_or(u64::MAX);
    if total > 200_000 {
        return greedy_voice_lead(prev, &candidates, n_pcs);
    }

    let need_all_pcs = n_voices >= n_pcs;
    let mut best_cost = i64::MAX;
    let mut best: Vec<u8> = Vec::new();

    for mut assignment in 0..total {
        let mut voicing = Vec::with_capacity(n_voices);
        let mut used = vec![false; n_pcs];
        for cand in candidates.iter().take(n_voices) {
            let pc_idx = (assignment % n_pcs as u64) as usize;
            assignment /= n_pcs as u64;
            voicing.push(cand[pc_idx]);
            used[pc_idx] = true;
        }
        if need_all_pcs && !used.iter().all(|&u| u) {
            continue;
        }
        let cost: i64 = prev
            .iter()
            .zip(voicing.iter())
            .map(|(&p, &n)| (p as i64 - n as i64).abs())
            .sum();
        if cost < best_cost {
            best_cost = cost;
            best = voicing;
        }
    }

    if best.is_empty() {
        // Fallback: shouldn't happen given the search above, but stay safe.
        best = candidates[0].clone();
    }
    best.sort_unstable();
    best
}

/// Fallback used when the brute-force search space is too big. Assigns
/// each voice its own nearest pitch class greedily, then patches the
/// result so every chord tone appears at least once whenever we have
/// at least as many voices as pitch classes — the same `need_all_pcs`
/// invariant the brute-force path enforces.
fn greedy_voice_lead(prev: &[u8], candidates: &[Vec<u8>], n_pcs: usize) -> Vec<u8> {
    // 1. Per-voice greedy pick (and remember which PC index was chosen).
    let mut picks: Vec<(usize, u8)> = prev
        .iter()
        .enumerate()
        .map(|(v, &p)| {
            let row = &candidates[v];
            let (pc_idx, &midi) = row
                .iter()
                .enumerate()
                .min_by_key(|(_, &c)| (c as i32 - p as i32).abs())
                .expect("candidates row is non-empty");
            (pc_idx, midi)
        })
        .collect();

    // 2. If we have enough voices to cover every PC, substitute one of
    // the doubled voices with each missing PC. We pick the voice whose
    // greedy choice is cheapest to swap (smallest cost delta vs.
    // dropping its duplicate role) so the patch barely perturbs the
    // optimum the greedy pass found.
    if prev.len() >= n_pcs {
        let mut used = vec![false; n_pcs];
        for &(pc_idx, _) in &picks {
            if pc_idx < n_pcs {
                used[pc_idx] = true;
            }
        }
        for missing in (0..n_pcs).filter(|&i| !used[i]) {
            // Find the voice that has a duplicate role (its pc_idx is
            // also held by another voice) and whose row even has the
            // missing PC option. Among those, pick the one with the
            // smallest distance increase.
            let mut best: Option<(usize, i32)> = None;
            for (v, &(pc_idx, midi)) in picks.iter().enumerate() {
                let duplicate = picks
                    .iter()
                    .enumerate()
                    .any(|(o, &(opc, _))| o != v && opc == pc_idx);
                if !duplicate {
                    continue;
                }
                if let Some(&alt) = candidates[v].get(missing) {
                    let delta = (alt as i32 - prev[v] as i32).abs()
                        - (midi as i32 - prev[v] as i32).abs();
                    if best.is_none_or(|(_, d)| delta < d) {
                        best = Some((v, delta));
                    }
                }
            }
            if let Some((v, _)) = best {
                let alt = candidates[v][missing];
                picks[v] = (missing, alt);
            }
        }
    }

    let mut voicing: Vec<u8> = picks.into_iter().map(|(_, m)| m).collect();
    voicing.sort_unstable();
    voicing
}

