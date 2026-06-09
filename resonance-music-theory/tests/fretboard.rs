//! Fretboard voicing search, including upper-register positions
//! reached via `voicing_from`.

use resonance_music_theory::chord::ChordQuality;
use resonance_music_theory::{
    fretboard_voicing, fretboard_voicing_from, Chord, PitchClass, GUITAR_6, MAX_START_FRET,
};

fn a_major() -> Chord {
    Chord::new(PitchClass::A, ChordQuality::Maj)
}

#[test]
fn open_position_c_major_unchanged() {
    // Textbook open C shape: x-3-2-0-1-0 (low E muted below the root).
    let v = fretboard_voicing(&Chord::new(PitchClass::C, ChordQuality::Maj), &GUITAR_6);
    assert_eq!(
        v.frets,
        vec![None, Some(3), Some(2), Some(0), Some(1), Some(0)]
    );
    assert_eq!(v.start_fret, 0);
}

#[test]
fn min_start_yields_fifth_fret_barre() {
    // E-shape A-major barre at fret 5: 5-7-7-6-5-5.
    let v = fretboard_voicing_from(&a_major(), &GUITAR_6, 5);
    assert_eq!(
        v.frets,
        vec![Some(5), Some(7), Some(7), Some(6), Some(5), Some(5)]
    );
    assert_eq!(v.start_fret, 5);
}

#[test]
fn upper_register_voicing_above_fret_11_is_reachable() {
    // Second-octave A-major shape at fret 12: x-12-14-14-14-12 — every
    // sounding fret is above the old `start..=7` window cap (max fret
    // 11), which made positions like this unreachable.
    let v = fretboard_voicing_from(&a_major(), &GUITAR_6, 12);
    assert_eq!(
        v.frets,
        vec![None, Some(12), Some(14), Some(14), Some(14), Some(12)]
    );
    assert_eq!(v.start_fret, 12);
    let highest = v.frets.iter().filter_map(|f| *f).max().unwrap();
    assert!(highest > 11, "expected an upper-register fret, got {highest}");
}

#[test]
fn upper_register_voicings_are_fully_fretted() {
    // E major has three open-string chord tones on a guitar; a
    // `min_start > 0` request must not fall back to them.
    let v = fretboard_voicing_from(&Chord::new(PitchClass::E, ChordQuality::Maj), &GUITAR_6, 8);
    for fret in v.frets.iter().flatten() {
        assert!(
            (8..=12).contains(fret),
            "fret {fret} outside the requested 8..=12 window"
        );
    }
    assert!(v.frets.iter().any(|f| f.is_some()));
}

#[test]
fn min_start_clamps_to_max_start_fret() {
    let v = fretboard_voicing_from(&a_major(), &GUITAR_6, u8::MAX);
    for fret in v.frets.iter().flatten() {
        assert!(
            (MAX_START_FRET..=MAX_START_FRET + 4).contains(fret),
            "fret {fret} outside the clamped {MAX_START_FRET}..={} window",
            MAX_START_FRET + 4
        );
    }
    assert!(v.frets.iter().any(|f| f.is_some()));
}

#[test]
fn default_search_still_prefers_the_lowest_position() {
    // Raising the cap must not change `voicing()` output: a window at
    // start 0/1 always sounds every string (chord-tone gaps are < 6
    // semitones, except dim, whose fret-1 window still covers all
    // strings), so a higher start can never win the score tie-break.
    for root in 0..12u8 {
        let root_pc = PitchClass::from_semitone(root);
        for q in ChordQuality::ALL {
            let v = fretboard_voicing(&Chord::new(root_pc, q), &GUITAR_6);
            let highest = v.frets.iter().filter_map(|f| *f).max().unwrap_or(0);
            assert!(
                highest <= 5,
                "{root_pc}{q}: open-position search escaped to fret {highest}"
            );
        }
    }
}
