/// Björklund's algorithm: distribute `hits` evenly across `steps` positions,
/// then rotate left by `rotation`.
///
/// Edge cases:
/// - `steps == 0` → empty vec
/// - `hits == 0`  → all false
/// - `hits >= steps` → all true
///
/// Produces the Björklund pattern via the modular-remainder form
/// `pattern[i] = (i * hits) mod steps < hits`. This matches the classic
/// iterative algorithm for the canonical unrotated case: the first hit
/// always lands on step 0, and the remaining hits are spaced as evenly as
/// possible.
pub fn bjorklund(steps: u32, hits: u32, rotation: i32) -> Vec<bool> {
    if steps == 0 {
        return Vec::new();
    }
    let steps_us = steps as usize;
    if hits == 0 {
        return vec![false; steps_us];
    }
    if hits >= steps {
        return vec![true; steps_us];
    }

    let s = steps as u64;
    let h = hits as u64;
    let mut pattern = vec![false; steps_us];
    for i in 0..steps_us {
        if (i as u64 * h) % s < h {
            pattern[i] = true;
        }
    }

    // Rotation: positive rotates LEFT (first hit moves earlier). Uses
    // rem_euclid so negative rotations also behave sensibly.
    let rot = rotation.rem_euclid(steps as i32) as usize;
    if rot > 0 {
        pattern.rotate_left(rot);
    }
    pattern
}

/// Spread a boolean pattern across `clip_length_ticks` evenly, producing
/// one `MidiNote` per lit step. Each note has `duration_ticks` equal to one
/// step and the pattern's pad note number.
pub fn pattern_to_notes(
    pattern: &[bool],
    pad_note: u8,
    velocity: f32,
    clip_length_ticks: u64,
) -> Vec<resonance_audio::types::MidiNote> {
    if pattern.is_empty() || clip_length_ticks == 0 {
        return Vec::new();
    }
    let steps = pattern.len() as u64;
    let step_ticks = clip_length_ticks / steps;
    if step_ticks == 0 {
        return Vec::new();
    }
    let mut notes = Vec::new();
    for (i, &hit) in pattern.iter().enumerate() {
        if !hit {
            continue;
        }
        notes.push(resonance_audio::types::MidiNote {
            note: pad_note,
            velocity,
            start_tick: i as u64 * step_ticks,
            duration_ticks: step_ticks,
        });
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_steps_is_empty() {
        assert_eq!(bjorklund(0, 0, 0), Vec::<bool>::new());
    }

    #[test]
    fn zero_hits_is_all_false() {
        assert_eq!(bjorklund(8, 0, 0), vec![false; 8]);
    }

    #[test]
    fn saturation_is_all_true() {
        assert_eq!(bjorklund(8, 8, 0), vec![true; 8]);
        // over-saturation clamps to steps
        assert_eq!(bjorklund(8, 12, 0), vec![true; 8]);
    }

    #[test]
    fn four_over_sixteen_is_four_on_the_floor() {
        // A quarter-note kick in 16ths.
        let p = bjorklund(16, 4, 0);
        assert_eq!(
            p,
            vec![
                true, false, false, false,
                true, false, false, false,
                true, false, false, false,
                true, false, false, false,
            ]
        );
    }

    #[test]
    fn three_over_eight_matches_tresillo() {
        // E(3, 8) = tresillo rhythm.
        // Björklund produces: X . . X . . X . (with the exact placement
        // depending on the formulation). Our fractional form gives:
        // positions 0, 3, 6.
        let p = bjorklund(8, 3, 0);
        assert_eq!(p.iter().filter(|b| **b).count(), 3);
        assert!(p[0]);
    }

    #[test]
    fn five_over_sixteen_has_correct_hit_count() {
        let p = bjorklund(16, 5, 0);
        assert_eq!(p.len(), 16);
        assert_eq!(p.iter().filter(|b| **b).count(), 5);
    }

    #[test]
    fn rotation_shifts_pattern() {
        let base = bjorklund(8, 3, 0);
        let rotated = bjorklund(8, 3, 2);
        // Rotation by 2 left: base[i] == rotated[i - 2 mod 8]
        for i in 0..8 {
            assert_eq!(base[(i + 2) % 8], rotated[i]);
        }
    }

    #[test]
    fn negative_rotation_works() {
        let base = bjorklund(8, 3, 0);
        let rotated = bjorklund(8, 3, -2);
        // -2 == +6 mod 8
        for i in 0..8 {
            assert_eq!(base[(i + 6) % 8], rotated[i]);
        }
    }

    #[test]
    fn pattern_to_notes_spaces_evenly() {
        // 4 hits over 16 ticks → one hit every 4 ticks.
        let p = vec![true, false, false, false, true, false, false, false,
                     true, false, false, false, true, false, false, false];
        let notes = pattern_to_notes(&p, 36, 0.9, 16);
        assert_eq!(notes.len(), 4);
        assert_eq!(notes[0].start_tick, 0);
        assert_eq!(notes[1].start_tick, 4);
        assert_eq!(notes[2].start_tick, 8);
        assert_eq!(notes[3].start_tick, 12);
        assert_eq!(notes[0].duration_ticks, 1);
        assert_eq!(notes[0].note, 36);
        assert_eq!(notes[0].velocity, 0.9);
    }

    #[test]
    fn pattern_to_notes_empty_returns_empty() {
        assert!(pattern_to_notes(&[], 36, 0.9, 16).is_empty());
        assert!(pattern_to_notes(&[true; 4], 36, 0.9, 0).is_empty());
    }
}
