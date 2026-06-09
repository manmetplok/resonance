// Harmony post-processing: align raw motif pitches to the current chord
// (chord-tone on strong beats, scale-tone on weak beats) and fill big
// leaps with stepwise passing tones.

use crate::chord::Chord;
use crate::scale::Scale;

use super::super::bass::step_scale;
use super::super::motif_bass::chord_tones_in_register;
use super::super::GeneratedNote;

/// Align a MIDI note to the current harmony based on beat strength.
pub(in crate::derive) fn align_to_harmony(
    raw_midi: u8,
    beat_position: u64,
    tpb: u64,
    chord: Chord,
    scale: Option<Scale>,
    register: (u8, u8),
) -> u8 {
    let chord_tones = chord_tones_in_register(chord, register);
    if chord_tones.is_empty() {
        return raw_midi.clamp(register.0, register.1);
    }

    // Strong beat: position is a multiple of 2 beats.
    let is_strong = tpb > 0 && beat_position.is_multiple_of(2 * tpb);

    if is_strong {
        // Must be a chord tone.
        if chord_tones.contains(&raw_midi) {
            return raw_midi;
        }
        return nearest_in_set(raw_midi, &chord_tones);
    }

    // Weak beat: allow scale tones.
    if let Some(scale) = scale {
        if scale.contains(raw_midi) {
            return raw_midi;
        }
        // Snap to nearest scale tone in register.
        let up = step_scale(&scale, raw_midi, 1);
        let down = step_scale(&scale, raw_midi, -1);
        let d_up = (up as i16 - raw_midi as i16).unsigned_abs() as u8;
        let d_down = (down as i16 - raw_midi as i16).unsigned_abs() as u8;
        let snapped = if d_up <= d_down { up } else { down };
        return snapped.clamp(register.0, register.1);
    }

    // No scale: snap to chord tone.
    nearest_in_set(raw_midi, &chord_tones)
}

/// Find the nearest value in a sorted set to the target.
pub(super) fn nearest_in_set(target: u8, set: &[u8]) -> u8 {
    let mut best = set[0];
    let mut best_dist = (target as i16 - best as i16).unsigned_abs();
    for &v in &set[1..] {
        let dist = (target as i16 - v as i16).unsigned_abs();
        if dist < best_dist {
            best = v;
            best_dist = dist;
        }
    }
    best
}

/// Post-processing: resolve large leaps (>5 semitones) with stepwise
/// fill notes in the opposite direction.
///
/// Complexity note: the `Vec::insert` calls below are O(n) each,
/// making the pass O(n²) in the worst case. That's deliberate — it
/// runs per phrase, and a phrase is ~16 notes (a handful of chords ×
/// 2-6 motif notes), so a linear rebuild into a second Vec would cost
/// more in code than it saves in time. Revisit only if phrases ever
/// grow by orders of magnitude.
pub(super) fn apply_gap_fill(notes: &mut Vec<GeneratedNote>, scale: &Scale, register: (u8, u8)) {
    let mut i = 0;
    while i + 1 < notes.len() {
        let leap = notes[i + 1].note as i16 - notes[i].note as i16;
        if leap.unsigned_abs() > 5 {
            let fill_dir: i32 = if leap > 0 { -1 } else { 1 };
            // Check if next notes already resolve the leap.
            let already_filled = (i + 2 < notes.len()) && {
                let next_step = notes[i + 2].note as i16 - notes[i + 1].note as i16;
                (fill_dir > 0 && next_step > 0) || (fill_dir < 0 && next_step < 0)
            };
            if !already_filled {
                // Insert 1-2 fill notes by splitting the post-leap note's duration.
                let fill_count = if leap.unsigned_abs() > 7 { 2 } else { 1 };
                let post = &notes[i + 1];
                if post.duration_ticks > fill_count as u64 * 60 {
                    let fill_dur = post.duration_ticks / (fill_count as u64 + 1);
                    let mut fill_notes = Vec::new();
                    let mut cur = post.note;
                    let orig_start = post.start_tick;
                    for f in 0..fill_count {
                        cur = step_scale(scale, cur, fill_dir);
                        cur = cur.clamp(register.0, register.1);
                        fill_notes.push(GeneratedNote {
                            note: cur,
                            velocity: post.velocity * 0.9,
                            start_tick: orig_start + post.duration_ticks - (fill_count - f) as u64 * fill_dur,
                            duration_ticks: fill_dur,
                        });
                    }
                    // Shorten the post-leap note.
                    notes[i + 1].duration_ticks -= fill_count as u64 * fill_dur;
                    let insert_pos = i + 2;
                    for (j, note) in fill_notes.into_iter().enumerate() {
                        notes.insert(insert_pos + j, note);
                    }
                    i += 1 + fill_count; // skip past inserted notes
                    continue;
                }
            }
        }
        i += 1;
    }
}
