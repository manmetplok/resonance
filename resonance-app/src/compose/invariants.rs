use super::{ChordState, SectionDefinitionState, SectionPlacementState};

/// Returns true if a new placement at `[start_bar, start_bar + length_bars)`
/// would overlap any existing placement (given the length of each definition).
/// `ignore_placement_id` lets the caller exclude the placement being moved.
pub fn placement_overlaps(
    placements: &[SectionPlacementState],
    definitions: &[SectionDefinitionState],
    start_bar: u32,
    length_bars: u32,
    ignore_placement_id: Option<u64>,
) -> bool {
    let new_end = start_bar + length_bars;
    for p in placements {
        if Some(p.id) == ignore_placement_id {
            continue;
        }
        let Some(def) = definitions.iter().find(|d| d.id == p.definition_id) else {
            continue;
        };
        let p_end = p.start_bar + def.length_bars;
        if start_bar < p_end && p.start_bar < new_end {
            return true;
        }
    }
    false
}

/// Returns true if a chord slot `[start_beat, start_beat + duration_beats)`
/// would overlap any existing chord in the section, excluding the one being
/// moved.
pub fn chord_overlaps(
    chords: &[ChordState],
    start_beat: u32,
    duration_beats: u32,
    ignore_chord_id: Option<u64>,
) -> bool {
    let new_end = start_beat + duration_beats;
    for c in chords {
        if Some(c.id) == ignore_chord_id {
            continue;
        }
        let c_end = c.start_beat + c.duration_beats;
        if start_beat < c_end && c.start_beat < new_end {
            return true;
        }
    }
    false
}

/// Returns true if the chord slot stays within the section's total beat span.
pub fn chord_fits_in_section(
    start_beat: u32,
    duration_beats: u32,
    section_length_bars: u32,
    time_sig_num: u8,
) -> bool {
    let section_beats = section_length_bars * time_sig_num as u32;
    duration_beats >= 1 && start_beat + duration_beats <= section_beats
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_music_theory::{Chord, ChordQuality, PitchClass};

    fn def(id: u64, length_bars: u32) -> SectionDefinitionState {
        SectionDefinitionState {
            id,
            name: format!("Def{id}"),
            color: [0, 0, 0],
            length_bars,
            chords: vec![],
            scale: None,
            progression_seed: 0,
            generate_params: crate::compose::GenerateParams::default(),
        }
    }

    fn placement(id: u64, definition_id: u64, start_bar: u32) -> SectionPlacementState {
        SectionPlacementState {
            id,
            definition_id,
            start_bar,
        }
    }

    fn chord(id: u64, start: u32, dur: u32) -> ChordState {
        ChordState {
            id,
            start_beat: start,
            duration_beats: dur,
            chord: Chord::new(PitchClass::C, ChordQuality::Maj),
        }
    }

    #[test]
    fn disjoint_placements_do_not_overlap() {
        let defs = vec![def(1, 8)];
        let placements = vec![placement(10, 1, 0)];
        assert!(!placement_overlaps(&placements, &defs, 8, 8, None));
        assert!(!placement_overlaps(&placements, &defs, 16, 4, None));
    }

    #[test]
    fn overlapping_placements_detected() {
        let defs = vec![def(1, 8)];
        let placements = vec![placement(10, 1, 4)]; // occupies 4..12
        assert!(placement_overlaps(&placements, &defs, 0, 8, None)); // 0..8 overlaps 4..12
        assert!(placement_overlaps(&placements, &defs, 11, 4, None));
        assert!(!placement_overlaps(&placements, &defs, 12, 4, None)); // touches but ok
    }

    #[test]
    fn ignore_self_when_moving() {
        let defs = vec![def(1, 4)];
        let placements = vec![placement(10, 1, 0), placement(11, 1, 4)];
        // Moving placement 10 to bar 4 would collide with placement 11, but
        // if we tell the checker to ignore placement 10's own footprint, the
        // only question is whether it clashes with 11.
        assert!(placement_overlaps(&placements, &defs, 4, 4, Some(10)));
        assert!(!placement_overlaps(&placements, &defs, 8, 4, Some(10)));
    }

    #[test]
    fn chord_overlap_basic() {
        let chords = vec![chord(1, 0, 4), chord(2, 8, 4)];
        assert!(!chord_overlaps(&chords, 4, 4, None));
        assert!(chord_overlaps(&chords, 2, 4, None));
        assert!(chord_overlaps(&chords, 6, 4, None));
    }

    #[test]
    fn chord_fits_inside_section() {
        assert!(chord_fits_in_section(0, 4, 4, 4)); // 4-bar section, 16 beats, 0..4 fits
        assert!(chord_fits_in_section(12, 4, 4, 4)); // 12..16 ends exactly at section end
        assert!(!chord_fits_in_section(14, 4, 4, 4)); // 14..18 exceeds 16
        assert!(!chord_fits_in_section(0, 0, 4, 4)); // duration must be >= 1
    }
}
